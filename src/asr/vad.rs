//! VAD Silero: acha os trechos de fala, junta só eles para o whisper e leva os tempos do
//! áudio juntado de volta para o original.
//!
//! Feito aqui, e não pelo `vad` dos parâmetros do whisper: o `whisper_full_with_state`, que
//! o whisper-rs chama, ignora esse parâmetro (só o `whisper_full` aplica o VAD, no estado
//! interno do contexto, que o whisper-rs não cria). A junção segue a do whisper.cpp: cada
//! trecho leva 0,1 s do original depois do fim detectado e ganha 0,1 s de silêncio antes do
//! próximo. O mapa de tempo é exato, porque cada trecho entra inteiro.

use std::ffi::{c_int, c_void};
use std::ops::Range;
use std::ptr::NonNull;

use whisper_rs::whisper_rs_sys as sys;

use super::log;

/// Amostras por milissegundo, a 16 kHz.
const PER_MS: usize = 16;
/// Amostras por centésimo de segundo, a unidade de tempo do whisper.cpp.
const PER_CS: usize = 160;
/// Original que cada trecho leva depois do fim detectado: 0,1 s.
const OVERLAP: usize = 1_600;
/// Silêncio entre dois trechos no áudio juntado: 0,1 s.
const GAP: usize = 1_600;

pub struct Vad(NonNull<sys::whisper_vad_context>);

// SAFETY: o contexto só é usado por quem o possui (`&mut self`), uma thread por vez.
unsafe impl Send for Vad {}

impl Vad {
    /// Carrega o modelo da memória: o embutido no exe serve direto, sem arquivo.
    pub fn load(model: &[u8], threads: i32) -> Result<Self, String> {
        let mut reader = Reader {
            data: model,
            pos: 0,
        };
        let mut loader = sys::whisper_model_loader {
            context: (&raw mut reader).cast(),
            read: Some(read),
            eof: Some(eof),
            close: Some(close),
        };
        let (ptr, _) = log::capture(|| {
            // SAFETY: o loader e o leitor vivem até o fim da chamada, que lê o modelo
            // inteiro antes de voltar.
            unsafe {
                let mut params = sys::whisper_vad_default_context_params();
                params.n_threads = threads;
                params.use_gpu = false;
                sys::whisper_vad_init_with_params(&mut loader, params)
            }
        });
        NonNull::new(ptr)
            .map(Self)
            .ok_or_else(|| "o modelo do VAD não carregou".to_string())
    }

    /// Os trechos com fala, em amostras de `samples`, em ordem e sem sobreposição.
    pub fn speech(&mut self, samples: &[f32]) -> Result<Vec<Range<usize>>, String> {
        if samples.is_empty() {
            return Ok(Vec::new());
        }
        let n = c_int::try_from(samples.len()).map_err(|_| "áudio longo demais para o VAD")?;
        // SAFETY: contexto válido; o whisper.cpp só lê `samples` durante a chamada.
        let segments = unsafe {
            sys::whisper_vad_segments_from_samples(
                self.0.as_ptr(),
                sys::whisper_vad_default_params(),
                samples.as_ptr(),
                n,
            )
        };
        if segments.is_null() {
            return Err("o VAD não analisou o áudio".into());
        }
        // Os tempos vêm em centésimos; o último trecho pode passar do fim do áudio.
        let at = |cs: f32| from_cs(cs as i64).min(samples.len());
        // SAFETY: `segments` vale até o free logo abaixo.
        let spans = unsafe {
            let spans = (0..sys::whisper_vad_segments_n_segments(segments))
                .map(|i| {
                    at(sys::whisper_vad_segments_get_segment_t0(segments, i))
                        ..at(sys::whisper_vad_segments_get_segment_t1(segments, i))
                })
                .filter(|span| !span.is_empty())
                .collect();
            sys::whisper_vad_free_segments(segments);
            spans
        };
        Ok(spans)
    }
}

impl Drop for Vad {
    fn drop(&mut self) {
        // SAFETY: o ponteiro veio do init e é solto uma vez só.
        unsafe { sys::whisper_vad_free(self.0.as_ptr()) }
    }
}

/// O modelo em memória, lido pelo loader do whisper.cpp.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

unsafe extern "C" fn read(ctx: *mut c_void, out: *mut c_void, size: usize) -> usize {
    // SAFETY: `ctx` é o `Reader` de `Vad::load` e `out` tem espaço para `size` bytes.
    let (reader, out) = unsafe {
        (
            &mut *ctx.cast::<Reader>(),
            std::slice::from_raw_parts_mut(out.cast::<u8>(), size),
        )
    };
    let rest = &reader.data[reader.pos..];
    let n = rest.len().min(size);
    out[..n].copy_from_slice(&rest[..n]);
    // Num modelo truncado, o que falta vem zerado em vez de lixo.
    out[n..].fill(0);
    reader.pos += n;
    n
}

unsafe extern "C" fn eof(ctx: *mut c_void) -> bool {
    // SAFETY: como em `read`.
    let reader = unsafe { &*ctx.cast::<Reader>() };
    reader.pos >= reader.data.len()
}

unsafe extern "C" fn close(_: *mut c_void) {}

/// O áudio só com a fala, para o whisper, e o mapa de volta para o original. `spans` vem em
/// ordem, como o VAD devolve; uma sobreposição é cortada.
pub fn splice(samples: &[f32], spans: &[Range<usize>]) -> (Vec<f32>, TimeMap) {
    let mut audio = Vec::new();
    let mut pieces: Vec<Piece> = Vec::new();
    for (i, span) in spans.iter().enumerate() {
        let next = spans.get(i + 1).map_or(samples.len(), |s| s.start);
        let done = pieces.last().map_or(0, |p| p.from + p.len);
        let start = span.start.max(done).min(samples.len());
        let end = (span.end + OVERLAP).min(next).min(samples.len());
        if start >= end {
            continue;
        }
        if !pieces.is_empty() {
            audio.resize(audio.len() + GAP, 0.0);
        }
        pieces.push(Piece {
            at: audio.len(),
            from: start,
            len: end - start,
        });
        audio.extend_from_slice(&samples[start..end]);
    }
    (audio, TimeMap { pieces, offset: 0 })
}

/// Leva um tempo do áudio juntado de volta para o original.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimeMap {
    pieces: Vec<Piece>,
    /// Onde o bloco começa no original, em amostras: somado a todo tempo devolvido.
    offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Piece {
    /// Início no áudio juntado, em amostras.
    at: usize,
    /// Início no original.
    from: usize,
    len: usize,
}

/// Quanto um segmento pode encostar num trecho sem que conte como fala dele: 0,25 s. O fim
/// de um trecho tem até isso sem fala (a margem do VAD e o 0,1 s extra), e o whisper, que no
/// áudio juntado só vê uma pausa curta, costuma começar ali o segmento seguinte, mesmo
/// quando a fala dele só vem depois de uma pausa longa no original.
const SLIVER: usize = 4_000;

impl TimeMap {
    /// O mesmo mapa para um bloco que começa em `offset` amostras do original.
    pub fn offset(self, offset: usize) -> Self {
        Self { offset, ..self }
    }

    /// O início e o fim de um segmento, de centésimos no áudio juntado para ms no original.
    /// Um segmento que atravessa o silêncio entre dois trechos começa no seguinte se só
    /// encosta no anterior, e termina no anterior se só encosta no seguinte: assim a pausa
    /// entre dois segmentos é a do original.
    pub fn segment_ms(&self, t0: i64, t1: i64) -> (u64, u64) {
        let a = from_cs(t0);
        let b = from_cs(t1).max(a);
        let mut start = self.original(a, true);
        if let Some(k) = self.piece_at(a)
            && let Some(next) = self.pieces.get(k + 1)
            && b > next.at
            && self.pieces[k].at + self.pieces[k].len - a < SLIVER
        {
            start = next.from;
        }
        let mut end = self.original(b, false);
        if let Some(m) = self.piece_at(b)
            && m > 0
            && a < self.pieces[m].at
            && b - self.pieces[m].at < SLIVER
        {
            let previous = self.pieces[m - 1];
            end = previous.from + previous.len;
        }
        let ms = |sample: usize| ((sample + self.offset) / PER_MS) as u64;
        let (start, end) = (ms(start), ms(end));
        (start, end.max(start))
    }

    /// O trecho que contém a amostra do áudio juntado.
    fn piece_at(&self, sample: usize) -> Option<usize> {
        let i = self
            .pieces
            .partition_point(|p| p.at <= sample)
            .checked_sub(1)?;
        let p = self.pieces[i];
        (sample < p.at + p.len).then_some(i)
    }

    /// A amostra no original. No silêncio entre dois trechos, um início vai para o começo
    /// do seguinte e um fim, para o fim do anterior.
    fn original(&self, sample: usize, start: bool) -> usize {
        let after = self.pieces.partition_point(|p| p.at <= sample);
        let Some(piece) = after.checked_sub(1).map(|i| self.pieces[i]) else {
            return self.pieces.first().map_or(sample, |p| p.from);
        };
        if sample < piece.at + piece.len {
            return piece.from + (sample - piece.at);
        }
        match self.pieces.get(after) {
            Some(next) if start => next.from,
            _ => piece.from + piece.len,
        }
    }
}

/// Centésimos de segundo em amostras; tempo negativo vale 0.
fn from_cs(cs: i64) -> usize {
    usize::try_from(cs).unwrap_or(0).saturating_mul(PER_CS)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    /// Áudio em que cada amostra vale o próprio índice: dá para ver de onde veio cada pedaço.
    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32).collect()
    }

    #[test]
    fn junta_os_trechos_com_silencio_entre_eles() {
        let samples = ramp(100_000);
        let (audio, map) = splice(&samples, &[10_000..20_000, 60_000..70_000]);
        // Cada trecho vai 0,1 s além do fim detectado.
        assert_eq!(audio.len(), 11_600 + GAP + 11_600);
        assert_eq!(audio[0], 10_000.0);
        assert_eq!(audio[11_599], 21_599.0);
        assert!(audio[11_600..11_600 + GAP].iter().all(|&s| s == 0.0));
        assert_eq!(audio[11_600 + GAP], 60_000.0);

        // Dentro de um trecho: 0,5 s no juntado = 10 000 + 8 000 amostras = 1,125 s.
        assert_eq!(map.segment_ms(50, 60), (1_125, 1_225));
        // Atravessa o silêncio pegando bem dos dois trechos: os tempos são os de cada lado.
        assert_eq!(map.segment_ms(10, 110), (725, 4_025));
        // Só encosta no fim do primeiro trecho: começa no segundo.
        assert_eq!(map.segment_ms(70, 110), (3_750, 4_025));
        // Só encosta no começo do segundo trecho: termina no primeiro.
        assert_eq!(map.segment_ms(10, 90), (725, 1_350));
        // Inteiro no silêncio inserido.
        assert_eq!(map.segment_ms(77, 78), (3_750, 3_750));
        // Antes do primeiro e depois do último trecho.
        assert_eq!(map.segment_ms(-5, 0), (625, 625));
        assert_eq!(map.segment_ms(150, 1_000_000), (4_425, 4_475));
    }

    #[test]
    fn pausa_longa_do_original_volta_entre_os_segmentos() {
        // O `pausas.wav`: duas frases com 3 s de silêncio, como o VAD as acha.
        let samples = vec![0.0; 210_673];
        let (_, map) = splice(&samples, &[3_040..61_440, 124_480..198_560]);
        // O whisper (base) começa o segundo segmento onde o primeiro termina, 0,11 s antes
        // do fim do primeiro trecho no áudio juntado.
        assert_eq!(map.segment_ms(0, 364), (190, 3_830));
        assert_eq!(map.segment_ms(364, 850), (7_780, 12_430));
        // O simétrico: o primeiro segmento termina 0,02 s dentro do segundo trecho.
        assert_eq!(map.segment_ms(0, 387), (190, 3_940));
        assert_eq!(map.segment_ms(387, 850), (7_800, 12_430));
    }

    #[test]
    fn o_trecho_extra_nao_invade_o_proximo() {
        let samples = ramp(50_000);
        let (audio, _) = splice(&samples, &[0..10_000, 10_500..20_000, 19_000..30_000]);
        // 0..10 500 (o extra para no próximo), 10 500..19 000, 19 000..31 600: nada repetido.
        assert_eq!(audio.len(), 10_500 + GAP + 8_500 + GAP + 12_600);
        let speech: Vec<f32> = audio.iter().copied().filter(|&s| s != 0.0).collect();
        assert!(speech.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn sem_fala_nao_ha_audio() {
        let (audio, map) = splice(&ramp(1_000), &[]);
        assert!(audio.is_empty());
        assert_eq!(map, TimeMap::default());
        assert_eq!(map.segment_ms(100, 200), (1_000, 2_000));
    }

    /// Tamanho do áudio, trechos de fala em ordem e fronteiras de segmentos em centésimos.
    fn case() -> impl Strategy<Value = (usize, Vec<Range<usize>>, Vec<i64>)> {
        (1_000usize..200_000).prop_flat_map(|len| {
            let cuts = prop::collection::vec(0..len, 0..20);
            let bounds = prop::collection::vec(-10i64..(len / PER_CS) as i64 + 10, 1..20);
            (Just(len), cuts, bounds).prop_map(|(len, mut cuts, mut bounds)| {
                cuts.sort_unstable();
                cuts.dedup();
                bounds.sort_unstable();
                let spans = cuts
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|[a, b]| *a..*b)
                    .collect();
                (len, spans, bounds)
            })
        })
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init, reason = "um trecho de fala só")]
    fn bloco_soma_o_proprio_inicio() {
        let samples = ramp(100_000);
        let (_, map) = splice(&samples, &[10_000..20_000, 60_000..70_000]);
        let shifted = map.clone().offset(16 * 30_000);
        for (t0, t1) in [(50, 60), (10, 110), (70, 110), (-5, 0), (150, 1_000_000)] {
            let (start, end) = map.segment_ms(t0, t1);
            assert_eq!(shifted.segment_ms(t0, t1), (start + 30_000, end + 30_000));
        }
        // Um início fora do múltiplo de 1 ms arredonda junto com o tempo do trecho.
        assert_eq!(map.offset(8).segment_ms(50, 60), (1_125, 1_225));
        let (_, map) = splice(&samples, &[10_008..20_000]);
        assert_eq!(map.offset(8).segment_ms(0, 1), (626, 636));
    }

    proptest! {
        #[test]
        fn mapa_volta_para_o_original((len, spans, bounds) in case()) {
            let samples = ramp(len);
            let (audio, map) = splice(&samples, &spans);
            let speech: usize = map.pieces.iter().map(|p| p.len).sum();
            let gaps = map.pieces.len().saturating_sub(1) * GAP;
            prop_assert_eq!(audio.len(), speech + gaps);
            let mut last = (0, 0);
            for p in &map.pieces {
                // Cada trecho entra inteiro e na ordem do original.
                prop_assert_eq!(&audio[p.at..p.at + p.len], &samples[p.from..p.from + p.len]);
                prop_assert!(p.from >= last.1);
                last = (p.at, p.from + p.len);
            }
            // Segmentos seguidos, como o whisper os dá: nenhum termina antes de começar, os
            // tempos nunca voltam e nada passa do fim do original.
            let total_ms = (len / PER_MS) as u64;
            let mut previous = (0, 0);
            for w in bounds.windows(2) {
                let (start, end) = map.segment_ms(w[0], w[1]);
                prop_assert!(start <= end);
                prop_assert!(start >= previous.0 && end >= previous.1, "{:?}", (w, previous));
                if !map.pieces.is_empty() {
                    prop_assert!(end <= total_ms);
                }
                previous = (start, end);
            }
        }
    }
}
