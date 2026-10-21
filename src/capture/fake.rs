//! Fonte falsa, para os testes e o `--fake-audio` (feature `test-support`): cada trilha aberta
//! ganha uma thread que entrega pacotes como um driver, seguindo um roteiro: o sinal (tom,
//! impulsos nos mesmos instantes nas duas trilhas, ou amostras dadas), a taxa e os canais, o
//! tamanho do pacote, jitter na entrega e no timestamp, lacunas (pacotes que não chegam),
//! deriva do relógio do dispositivo e um erro num instante dado. O relógio pode correr mais
//! rápido que o real, para os testes longos.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::source::{
    AudioSource, Device, Devices, Error, EventKind, Events, Format, Opened, Producer, RING_SECS,
    Track, ring,
};

#[derive(Clone, Debug)]
pub enum Signal {
    Silence,
    Tone {
        freq: f32,
        amp: f32,
    },
    /// Um pico de `amp` em cada múltiplo de `every_ms` do relógio: as duas trilhas têm os
    /// picos nos mesmos instantes, o que mede o alinhamento.
    Impulses {
        every_ms: u64,
        amp: f32,
    },
    /// Mono, na taxa da trilha, desde a abertura; depois, silêncio.
    Samples(Arc<Vec<f32>>),
}

#[derive(Clone, Debug)]
pub struct TrackScript {
    pub name: String,
    pub rate: u32,
    pub channels: u16,
    pub packet_ms: u32,
    pub signal: Signal,
    /// Intervalos do relógio (ms) em que os pacotes não chegam.
    pub gaps: Vec<Range<u64>>,
    /// Atraso aleatório na entrega, até este tanto.
    pub delivery_jitter_ms: u32,
    /// Erro aleatório no timestamp, até ± este tanto.
    pub timestamp_jitter_us: u32,
    /// O relógio do dispositivo anda `ppm` partes por milhão mais rápido que o de verdade.
    pub ppm: i32,
    /// No instante dado (ms do relógio), o stream aberto antes dele avisa e para de entregar.
    pub fail: Option<(u64, EventKind)>,
    /// O `open` falha com este erro.
    pub open_error: Option<Error>,
}

impl TrackScript {
    pub fn new(name: &str, rate: u32, channels: u16, signal: Signal) -> Self {
        Self {
            name: name.into(),
            rate,
            channels,
            packet_ms: 10,
            signal,
            gaps: Vec::new(),
            delivery_jitter_ms: 0,
            timestamp_jitter_us: 0,
            ppm: 0,
            fail: None,
            open_error: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Script {
    pub mic: TrackScript,
    pub system: TrackScript,
    pub devices: Devices,
}

impl Script {
    /// Microfone a 48 kHz mono com um tom de 440 Hz; saída a 48 kHz estéreo com 660 Hz, como
    /// o WASAPI compartilhado entrega.
    pub fn tones() -> Self {
        Self {
            mic: TrackScript::new(
                "Microfone simulado",
                48_000,
                1,
                Signal::Tone {
                    freq: 440.0,
                    amp: 0.3,
                },
            ),
            system: TrackScript::new(
                "Alto-falantes simulados",
                48_000,
                2,
                Signal::Tone {
                    freq: 660.0,
                    amp: 0.2,
                },
            ),
            devices: devices(),
        }
    }

    fn track(&mut self, track: Track) -> &mut TrackScript {
        match track {
            Track::Mic => &mut self.mic,
            Track::System => &mut self.system,
        }
    }
}

/// Os dispositivos simulados: um deles Bluetooth, para o aviso dos Ajustes.
pub fn devices() -> Devices {
    let device = |id: &str, name: &str, default: bool, bluetooth: bool| Device {
        id: id.into(),
        name: name.into(),
        default,
        bluetooth,
    };
    Devices {
        inputs: vec![
            device("falso:mic", "Microfone simulado", true, false),
            device("falso:fone-bt", "Fone Bluetooth simulado", false, true),
        ],
        outputs: vec![device(
            "falso:saida",
            "Alto-falantes simulados",
            true,
            false,
        )],
    }
}

/// O relógio da fonte, que pode correr `speed` vezes mais rápido que o real.
#[derive(Clone, Copy)]
struct Clock {
    start: Instant,
    speed: f64,
}

/// O relógio começa em 1 s: nenhum timestamp é zero.
const BASE_NS: u64 = 1_000_000_000;

impl Clock {
    fn now_ns(&self) -> u64 {
        BASE_NS + (self.start.elapsed().as_nanos() as f64 * self.speed) as u64
    }

    fn sleep_until(&self, t_ns: u64, stop: &AtomicBool) {
        loop {
            let now = self.now_ns();
            if now >= t_ns || stop.load(Ordering::Relaxed) {
                return;
            }
            let real = ((t_ns - now) as f64 / self.speed) as u64;
            thread::sleep(Duration::from_nanos(real.min(5_000_000)));
        }
    }
}

pub struct FakeSource {
    clock: Clock,
    script: Mutex<Script>,
    opens: [AtomicU32; 2],
}

impl FakeSource {
    /// Em tempo real.
    pub fn new(script: Script) -> Self {
        Self::with_speed(script, 1.0)
    }

    /// O relógio corre `speed` vezes mais rápido que o real.
    pub fn with_speed(script: Script, speed: f64) -> Self {
        Self {
            clock: Clock {
                start: Instant::now(),
                speed,
            },
            script: Mutex::new(script),
            opens: [AtomicU32::new(0), AtomicU32::new(0)],
        }
    }

    /// Muda o roteiro (vale para as próximas aberturas e para os streams já abertos que leem
    /// o erro programado).
    pub fn edit(&self, f: impl FnOnce(&mut Script)) {
        f(&mut self.script.lock().unwrap_or_else(PoisonError::into_inner));
    }

    /// Quantas vezes a trilha foi aberta (reaberturas contam).
    pub fn opens(&self, track: Track) -> u32 {
        self.opens[track.index()].load(Ordering::Relaxed)
    }

    fn script(&self) -> Script {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl AudioSource for FakeSource {
    fn devices(&self) -> Result<Devices, Error> {
        Ok(self.script().devices)
    }

    fn now_ns(&self) -> u64 {
        self.clock.now_ns()
    }

    fn open(&self, track: Track, _device: Option<&str>, events: Events) -> Result<Opened, Error> {
        let mut script = self.script();
        let t = script.track(track).clone();
        if let Some(e) = t.open_error {
            return Err(e);
        }
        let n = self.opens[track.index()].fetch_add(1, Ordering::Relaxed);
        // Com o relógio acelerado, o ring cobre o mesmo atraso real da thread de escrita que
        // o do app (pelo menos ~6 s de verdade), para uma máquina ocupada não perder pacotes.
        let secs = RING_SECS.max((self.clock.speed * 6.0).ceil() as u32);
        let (producer, consumer) = ring(
            Format {
                rate: t.rate,
                channels: t.channels,
            },
            secs,
        );
        let stop = Arc::new(AtomicBool::new(false));
        let clock = self.clock;
        let seed = 0x9E37_79B9_7F4A_7C15 ^ (u64::from(n) << 8) ^ track.index() as u64;
        let name = t.name.clone();
        let thread = {
            let stop = stop.clone();
            thread::Builder::new()
                .name(format!("scriba-audio-falso-{track:?}"))
                .spawn(move || deliver(&t, clock, producer, &events, &stop, seed))
                .expect("criando a thread do áudio falso")
        };
        Ok(Opened {
            consumer,
            name,
            streams: Box::new(Stream {
                stop,
                thread: Some(thread),
            }),
        })
    }
}

/// Fecha como um stream do cpal: o callback para quando o valor sai de cena.
struct Stream {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// xorshift64: aleatório reprodutível, sem crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// De −1 a 1.
    fn signed(&mut self) -> f64 {
        (self.next() % 2_000_001) as f64 / 1_000_000.0 - 1.0
    }
}

fn deliver(
    t: &TrackScript,
    clock: Clock,
    mut producer: Producer,
    events: &Events,
    stop: &AtomicBool,
    seed: u64,
) {
    let mut rng = Rng(seed | 1);
    let frames = (u64::from(t.rate) * u64::from(t.packet_ms) / 1_000).max(1);
    let true_rate = f64::from(t.rate) * (1.0 + f64::from(t.ppm) / 1e6);
    let frame_ns = |k: u64| (k as f64 / true_rate * 1e9) as u64;
    let t_open = clock.now_ns();
    let mut produced = 0u64;
    let mut buf = Vec::with_capacity(frames as usize * usize::from(t.channels));
    while !stop.load(Ordering::Relaxed) {
        let t_first = t_open + frame_ns(produced);
        let t_last = t_open + frame_ns(produced + frames);
        // O driver entrega o pacote depois que o último frame foi capturado.
        let jitter = if t.delivery_jitter_ms > 0 {
            rng.next() % (u64::from(t.delivery_jitter_ms) * 1_000_000)
        } else {
            0
        };
        clock.sleep_until(t_last + jitter, stop);
        if stop.load(Ordering::Relaxed) {
            return;
        }
        // Só o stream que já estava aberto no instante falha: o reaberto segue.
        if let Some((at_ms, kind)) = t.fail
            && t_open < at_ms * 1_000_000
            && t_first >= at_ms * 1_000_000
        {
            events.send(kind);
            // O stream morto não entrega mais nada; espera ser fechado.
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
            return;
        }
        let in_gap = t.gaps.iter().any(|g| g.contains(&(t_first / 1_000_000)));
        if !in_gap {
            buf.clear();
            for k in produced..produced + frames {
                let value = sample(
                    &t.signal,
                    k,
                    t_open + frame_ns(k),
                    t_open + frame_ns(k + 1),
                    t.rate,
                );
                buf.extend(std::iter::repeat_n(value, usize::from(t.channels)));
            }
            let ts_jitter = (rng.signed() * f64::from(t.timestamp_jitter_us) * 1_000.0) as i64;
            let t_ns = (t_first as i64 + ts_jitter).max(0) as u64;
            producer.push(t_ns, frames as u32, buf.iter().copied());
        }
        produced += frames;
    }
}

/// A amostra do frame `k`, capturado entre `t_ns` e `next_ns`.
fn sample(signal: &Signal, k: u64, t_ns: u64, next_ns: u64, rate: u32) -> f32 {
    match signal {
        Signal::Silence => 0.0,
        Signal::Tone { freq, amp } => {
            let secs = k as f64 / f64::from(rate);
            (secs * f64::from(*freq) * std::f64::consts::TAU).sin() as f32 * amp
        }
        Signal::Impulses { every_ms, amp } => {
            let every = every_ms * 1_000_000;
            // O frame que contém um múltiplo de `every`.
            if t_ns.div_ceil(every) * every < next_ns {
                *amp
            } else {
                0.0
            }
        }
        Signal::Samples(samples) => samples.get(k as usize).copied().unwrap_or(0.0),
    }
}
