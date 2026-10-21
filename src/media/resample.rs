//! Reamostrador de áudio mono f32 (o swr do FFmpeg), para a gravação: cada trilha sai do
//! dispositivo na taxa dele e vai para o WAV a 32 kHz. Também corrige a deriva do relógio do
//! dispositivo aos poucos (`swr_set_compensation`), acrescentando ou tirando amostras ao longo
//! de vários segundos, sem clique.

use std::ptr;

use ffmpeg::format::{Sample, sample};
use ffmpeg::software::resampling;
use ffmpeg::{ChannelLayout, Dictionary, ffi};
use ffmpeg_next as ffmpeg;

pub struct Resampler {
    ctx: resampling::Context,
    input: u32,
    output: u32,
}

impl Resampler {
    /// De `input` Hz para `output` Hz, mono f32. A reamostragem fica sempre ligada, mesmo com
    /// as taxas iguais, para a compensação valer.
    pub fn new(input: u32, output: u32) -> Result<Self, ffmpeg::Error> {
        super::init();
        let mut options = Dictionary::new();
        options.set("flags", "res");
        let ctx = resampling::Context::get_with(
            Sample::F32(sample::Type::Packed),
            ChannelLayout::MONO,
            input,
            Sample::F32(sample::Type::Packed),
            ChannelLayout::MONO,
            output,
            options,
        )?;
        Ok(Self { ctx, input, output })
    }

    pub fn input_rate(&self) -> u32 {
        self.input
    }

    /// Reamostra `input` e acrescenta o resultado a `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), ffmpeg::Error> {
        self.convert(Some(input), out)
    }

    /// Entrega o que o reamostrador ainda guarda.
    pub fn flush(&mut self, out: &mut Vec<f32>) -> Result<(), ffmpeg::Error> {
        self.convert(None, out)
    }

    /// Amostras de saída que entraram e ainda não saíram.
    pub fn pending(&self) -> u64 {
        // SAFETY: o contexto é válido enquanto `self` existe; `swr_get_delay` só lê.
        let delay =
            unsafe { ffi::swr_get_delay(self.ctx.as_ptr().cast_mut(), i64::from(self.output)) };
        u64::try_from(delay).unwrap_or(0)
    }

    /// Acrescenta `delta` amostras de saída (tira, se negativo) ao longo das próximas
    /// `distance`; `delta` 0 desliga a correção.
    pub fn compensate(&mut self, delta: i32, distance: i32) -> Result<(), ffmpeg::Error> {
        let distance = if delta == 0 { 0 } else { distance.max(1) };
        // SAFETY: contexto válido e inicializado.
        let ret = unsafe { ffi::swr_set_compensation(self.ctx.as_mut_ptr(), delta, distance) };
        if ret < 0 {
            return Err(ffmpeg::Error::from(ret));
        }
        Ok(())
    }

    fn convert(&mut self, input: Option<&[f32]>, out: &mut Vec<f32>) -> Result<(), ffmpeg::Error> {
        let count = input.map_or(0, <[f32]>::len);
        let count = i32::try_from(count).map_err(|_| ffmpeg::Error::InvalidData)?;
        loop {
            // SAFETY: contexto válido; `swr_get_out_samples` só calcula.
            let room = unsafe { ffi::swr_get_out_samples(self.ctx.as_mut_ptr(), count) };
            if room < 0 {
                return Err(ffmpeg::Error::from(room));
            }
            // Um pouco de folga: a compensação pode pedir mais que a estimativa.
            let room = room as usize + 64;
            let start = out.len();
            out.resize(start + room, 0.0);
            let out_ptr = out[start..].as_mut_ptr().cast::<u8>();
            let in_ptr = input.map_or(ptr::null(), |i| i.as_ptr().cast::<u8>());
            let in_planes = [in_ptr];
            // SAFETY: mono packed: um plano de saída com `room` amostras livres e, na entrada,
            // um plano com `count` amostras (ou nenhum, para esvaziar).
            let written = unsafe {
                ffi::swr_convert(
                    self.ctx.as_mut_ptr(),
                    &out_ptr,
                    room as i32,
                    if input.is_some() {
                        in_planes.as_ptr()
                    } else {
                        ptr::null()
                    },
                    if input.is_some() { count } else { 0 },
                )
            };
            if written < 0 {
                out.truncate(start);
                return Err(ffmpeg::Error::from(written));
            }
            out.truncate(start + written as usize);
            // Ao esvaziar, repete até não sair mais nada; com entrada, uma chamada basta.
            if input.is_some() || written == 0 {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, secs: f64) -> Vec<f32> {
        let n = (f64::from(rate) * secs) as usize;
        (0..n)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5)
            .collect()
    }

    fn run(r: &mut Resampler, input: &[f32], chunk: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for c in input.chunks(chunk) {
            r.push(c, &mut out).unwrap();
        }
        r.flush(&mut out).unwrap();
        out
    }

    #[test]
    fn muda_a_taxa_sem_perder_duracao() {
        for (from, to) in [
            (48_000, 32_000),
            (44_100, 32_000),
            (16_000, 32_000),
            (32_000, 32_000),
        ] {
            let mut r = Resampler::new(from, to).unwrap();
            let out = run(&mut r, &tone(from, 2.0), 480);
            let expected = 2 * to as usize;
            assert!(
                out.len().abs_diff(expected) <= 2,
                "{from} → {to}: {} amostras",
                out.len()
            );
            // Depois de esvaziar, sobra só a latência do filtro (até ~1 ms).
            assert!(r.pending() <= u64::from(to) / 500, "{}", r.pending());
        }
    }

    #[test]
    fn compensacao_acrescenta_e_tira_amostras() {
        for delta in [320, -320] {
            let mut r = Resampler::new(48_000, 32_000).unwrap();
            r.compensate(delta, 32_000).unwrap();
            let out = run(&mut r, &tone(48_000, 2.0), 480);
            let expected = 64_000 + i64::from(delta);
            assert!(
                (out.len() as i64 - expected).abs() <= 8,
                "delta {delta}: {} amostras",
                out.len()
            );
        }
    }

    #[test]
    fn guarda_amostras_entre_as_chamadas() {
        let mut r = Resampler::new(48_000, 32_000).unwrap();
        let mut out = Vec::new();
        r.push(&tone(48_000, 0.01), &mut out).unwrap();
        assert!(r.pending() > 0 || out.len() == 320);
        assert_eq!(r.input_rate(), 48_000);
    }
}
