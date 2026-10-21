//! A parte pura da escrita: downmix para mono, f32 → i16, o nível RMS dos medidores e o
//! [`Aligner`], que prende cada trilha ao relógio dos pacotes (o QPC, o mesmo nas duas): a
//! amostra escrita no instante `t` fica na posição `(t − t0) × RATE`, com lacunas viradas em
//! silêncio e a deriva do relógio do dispositivo corrigida aos poucos.

/// Taxa dos WAVs das trilhas: o teto do `.m4a` da Fase 5 e o dobro do que o whisper usa.
pub const RATE: u32 = 32_000;

/// Acima disso, a diferença para o relógio vira silêncio (ou descarte) na hora: um buraco de
/// verdade (um pacote perdido já tem ~10 ms). Abaixo, a correção é aos poucos.
pub const GAP_MS: u64 = 8;
/// Uma lacuna maior que isso não é um pacote perdido: o computador dormiu (ou o dispositivo
/// devolveu um timestamp absurdo ao acordar). A gravação termina em vez de virar silêncio.
pub const SUSPEND_MS: u64 = 30_000;
/// Abaixo disso, nenhuma correção: o jitter dos timestamps não mexe no áudio.
pub const DEADBAND_MS: u64 = 2;
/// A correção da deriva se espalha por este tanto de áudio (0,4% de velocidade no máximo:
/// inaudível).
pub const COMPENSATION_SECS: u32 = 5;

/// Frames de áudio a `RATE` em `ms` milissegundos.
pub const fn frames_in_ms(ms: u64) -> u64 {
    ms * RATE as u64 / 1_000
}

/// Frames a `rate` Hz em `ns` nanossegundos (com sinal).
pub fn frames_at(ns: i128, rate: u32) -> i128 {
    ns * i128::from(rate) / 1_000_000_000
}

/// `frames` a `RATE` convertidos para a taxa `rate` do dispositivo.
pub fn to_rate(frames: u64, rate: u32) -> u64 {
    (u128::from(frames) * u128::from(rate) / u128::from(RATE)) as u64
}

/// Média dos canais de cada frame; os canais vêm intercalados.
pub fn downmix(data: &[f32], channels: u16, out: &mut Vec<f32>) {
    out.clear();
    let channels = usize::from(channels.max(1));
    if channels == 1 {
        out.extend_from_slice(data);
        return;
    }
    out.extend(
        data.chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32),
    );
}

/// Amostra f32 (−1..1) em i16, com limite e arredondamento.
pub fn to_i16(sample: f32) -> i16 {
    if sample.is_nan() {
        return 0;
    }
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// O piso do medidor, em dBFS: abaixo disso, a barra fica vazia.
pub const FLOOR_DB: f32 = -60.0;
/// Abaixo disso o microfone conta como mudo ("Microfone sem sinal").
pub const SILENT_DB: f32 = -50.0;

pub fn db(rms: f32) -> f32 {
    if rms <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * rms.log10()
    }
}

/// O RMS como fração da barra do medidor: −60 dBFS → 0, 0 dBFS → 1.
pub fn level(rms: f32) -> f32 {
    ((db(rms) - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// O que fazer antes de escrever um pacote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// O pacote chegou depois do esperado: tantos frames (a `RATE`) de silêncio antes dele.
    Silence(u64),
    /// O pacote começa antes do que já foi escrito: descarta tantos frames do começo dele.
    Drop(u64),
    /// Corrige aos poucos: acrescenta tantos frames (tira, se negativo); 0 desliga.
    Adjust(i64),
}

/// Prende uma trilha ao relógio. `position` é quanto já foi escrito mais o que está dentro do
/// reamostrador, em frames a `RATE`.
#[derive(Clone, Copy, Debug)]
pub struct Aligner {
    t0_ns: u64,
}

impl Aligner {
    /// `t0_ns` é o instante do Gravar: a posição 0 das duas trilhas.
    pub fn new(t0_ns: u64) -> Self {
        Self { t0_ns }
    }

    pub fn t0_ns(&self) -> u64 {
        self.t0_ns
    }

    /// Onde a amostra capturada em `t_ns` deve ficar, em frames a `RATE`.
    pub fn expected(&self, t_ns: u64) -> i128 {
        frames_at(i128::from(t_ns) - i128::from(self.t0_ns), RATE)
    }

    /// Quantos frames a trilha deve ter no instante `t_ns` (o fim da gravação).
    pub fn end(&self, t_ns: u64) -> u64 {
        self.expected(t_ns).max(0) as u64
    }

    /// A ação para o pacote capturado em `t_ns`, com a trilha em `position`.
    pub fn decide(&self, t_ns: u64, position: u64) -> Action {
        let error = self.expected(t_ns) - i128::from(position);
        let gap = i128::from(frames_in_ms(GAP_MS));
        let deadband = i128::from(frames_in_ms(DEADBAND_MS));
        if error > gap {
            Action::Silence(error as u64)
        } else if error < -gap {
            Action::Drop((-error) as u64)
        } else if error.abs() > deadband {
            Action::Adjust(error as i64)
        } else {
            Action::Adjust(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn downmix_faz_a_media_dos_canais() {
        let mut out = Vec::new();
        downmix(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2, &mut out);
        assert_eq!(out, [0.5, 0.5, 0.0]);
        downmix(&[0.1, 0.2], 1, &mut out);
        assert_eq!(out, [0.1, 0.2]);
        // Um frame incompleto no fim não vira amostra.
        downmix(&[0.3, 0.3, 0.3, 0.9], 3, &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn i16_com_limite_e_arredondamento() {
        assert_eq!(to_i16(0.0), 0);
        assert_eq!(to_i16(1.0), 32_767);
        assert_eq!(to_i16(-1.0), -32_767);
        assert_eq!(to_i16(2.5), 32_767);
        assert_eq!(to_i16(-7.0), -32_767);
        assert_eq!(to_i16(0.5), 16_384);
        assert_eq!(to_i16(-0.000_01), 0);
        assert_eq!(to_i16(f32::NAN), 0);
    }

    #[test]
    fn nivel_do_medidor() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5, -0.5]) - 0.5).abs() < 1e-6);
        assert_eq!(level(0.0), 0.0);
        assert_eq!(level(1.0), 1.0);
        // −30 dBFS fica no meio da barra.
        assert!((level(10f32.powf(-1.5)) - 0.5).abs() < 1e-3);
        assert!(db(0.001) < SILENT_DB);
    }

    #[test]
    fn decide_pela_distancia_do_relogio() {
        let a = Aligner::new(1_000_000_000);
        // 1 s depois do t0 = frame 32 000.
        assert_eq!(a.expected(2_000_000_000), 32_000);
        assert_eq!(a.decide(2_000_000_000, 32_000), Action::Adjust(0));
        // 1 ms: dentro da zona morta.
        assert_eq!(a.decide(2_001_000_000, 32_000), Action::Adjust(0));
        // 5 ms adiantado e atrasado: correção aos poucos.
        assert_eq!(a.decide(2_005_000_000, 32_000), Action::Adjust(160));
        assert_eq!(a.decide(1_995_000_000, 32_000), Action::Adjust(-160));
        // Um pacote perdido (10 ms): silêncio na hora.
        assert_eq!(a.decide(2_010_000_000, 32_000), Action::Silence(320));
        assert_eq!(a.decide(1_990_000_000, 32_000), Action::Drop(320));
        // Buraco de 1 s: silêncio.
        assert_eq!(a.decide(3_000_000_000, 32_000), Action::Silence(32_000));
        // Capturado antes do Gravar: descarta.
        assert_eq!(a.decide(500_000_000, 0), Action::Drop(16_000));
        assert_eq!(a.end(500_000_000), 0);
        assert_eq!(a.end(3_000_000_000), 64_000);
        assert_eq!(to_rate(32_000, 48_000), 48_000);
        assert_eq!(to_rate(320, 44_100), 441);
    }

    /// Um reamostrador ideal: `ratio` frames de saída por frame de entrada, com a compensação
    /// espalhada pela distância, como o swr.
    struct Ideal {
        rate: u32,
        carry: f64,
        delta: f64,
        left: f64,
        written: u64,
    }

    impl Ideal {
        fn push(&mut self, frames_in: u64) {
            let mut out = frames_in as f64 * f64::from(RATE) / f64::from(self.rate);
            if self.left > 0.0 {
                let share = out.min(self.left);
                out += self.delta * share / f64::from(RATE * COMPENSATION_SECS);
                self.left -= share;
            }
            let total = out + self.carry;
            self.written += total.floor() as u64;
            self.carry = total.fract();
        }

        fn apply(&mut self, action: Action, packet: u64) -> u64 {
            match action {
                Action::Silence(n) => {
                    self.push(to_rate(n, self.rate));
                    packet
                }
                Action::Drop(n) => packet.saturating_sub(to_rate(n, self.rate)),
                Action::Adjust(d) => {
                    self.delta = d as f64;
                    self.left = if d == 0 {
                        0.0
                    } else {
                        f64::from(RATE * COMPENSATION_SECS)
                    };
                    packet
                }
            }
        }
    }

    proptest! {
        /// Com o relógio do dispositivo derivando, jitter de até 5 ms nos timestamps e
        /// pacotes perdidos, a trilha nunca fica a mais de 10 ms do relógio.
        #[test]
        fn trilha_fica_presa_ao_relogio(
            rate in prop::sample::select(vec![16_000u32, 44_100, 48_000]),
            packet_ms in 3u64..30,
            ppm in -1_000i64..1_000,
            jitter in prop::collection::vec(-5_000i64..5_000, 400),
            lost in prop::collection::vec(prop::bool::weighted(0.05), 400),
        ) {
            let a = Aligner::new(7_000_000);
            let mut sim = Ideal { rate, carry: 0.0, delta: 0.0, left: 0.0, written: 0 };
            let frames = u64::from(rate) * packet_ms / 1_000;
            // O dispositivo produz `rate × (1 + ppm)` frames por segundo de verdade.
            let true_rate = f64::from(rate) * (1.0 + ppm as f64 / 1e6);
            let mut produced = 0u64;
            for i in 0..400 {
                let t_true = 7_000_000 + (produced as f64 / true_rate * 1e9) as u64;
                produced += frames;
                if lost[i] {
                    continue;
                }
                let t = (t_true as i64 + jitter[i]).max(0) as u64;
                let action = a.decide(t, sim.written);
                let keep = sim.apply(action, frames);
                sim.push(keep);
                // Depois de escrito, o fim do pacote deve estar perto do relógio verdadeiro.
                let t_end = 7_000_000 + (produced as f64 / true_rate * 1e9) as u64;
                let error = a.expected(t_end) - i128::from(sim.written);
                prop_assert!(
                    error.abs() <= i128::from(frames_in_ms(10)),
                    "pacote {i}: {error} frames ({action:?})"
                );
            }
        }
    }
}
