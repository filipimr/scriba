//! A thread de escrita (`scriba-gravacao`), dona dos streams: a cada ~20 ms esvazia os rings,
//! faz o downmix, alinha cada pacote ao relógio, reamostra para 32 kHz e grava os WAVs, com
//! `flush` a cada 5 s. Publica os níveis dos medidores, reabre um stream invalidado (troca do
//! padrão do Windows), checa o disco e o teto, e no fim completa as duas trilhas até o mesmo
//! instante.

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use super::Limits;
use super::align::{self, Action, Aligner, COMPENSATION_SECS, RATE};
use super::source::{AudioSource, Choice, Consumer, Event, EventKind, Events, Opened, Track};
use super::storage::{self, End, Manifest, RecState, Recording, TrackStats};
use crate::jobs::JobId;
use crate::media::Resampler;

/// Pausa entre as passadas pelos rings.
const TICK: Duration = Duration::from_millis(20);
/// O microfone mudo por este tanto vira o aviso "Microfone sem sinal".
pub const SILENT_SECS: u64 = 10;
/// Reaberturas seguidas de um stream que não para de ser invalidado: depois disso, a trilha
/// conta como perdida.
const MAX_REOPENS: u32 = 5;
const REOPEN_WINDOW_NS: u64 = 10_000_000_000;

pub const RUN: u8 = 0;
pub const STOP_USER: u8 = 1;
pub const STOP_EXIT: u8 = 2;

type Wav = hound::WavWriter<BufWriter<File>>;

/// O que a UI lê enquanto grava.
#[derive(Default)]
pub struct Shared {
    /// Os níveis (0..1) das trilhas, em bits de f32.
    levels: [AtomicU32; 2],
    pub elapsed_ms: AtomicU64,
    pub mic_silent: AtomicBool,
    pub xruns: AtomicU64,
    /// Os nomes dos dispositivos (mudam quando um stream é reaberto em outro).
    names: Mutex<[String; 2]>,
}

impl Shared {
    pub fn levels(&self) -> [f32; 2] {
        [0, 1].map(|i| f32::from_bits(self.levels[i].load(Ordering::Relaxed)))
    }

    fn set_level(&self, track: Track, level: f32) {
        self.levels[track.index()].store(level.to_bits(), Ordering::Relaxed);
    }

    pub fn names(&self) -> [String; 2] {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_name(&self, track: Track, name: String) {
        self.names.lock().unwrap_or_else(PoisonError::into_inner)[track.index()] = name;
    }
}

/// Uma trilha em gravação.
struct TrackWriter {
    track: Track,
    consumer: Consumer,
    resampler: Resampler,
    /// `None` só depois do fim.
    wav: Option<Wav>,
    /// Frames já no WAV.
    written: u64,
    /// A compensação em vigor no reamostrador.
    comp: i64,
    raw: Vec<f32>,
    mono: Vec<f32>,
    out: Vec<f32>,
    /// RMS das amostras escritas nesta passada.
    cycle_sum: f64,
    cycle_len: u64,
    level: f32,
    /// A última vez que a trilha passou do piso de silêncio.
    loud_at_ns: u64,
    stats: TrackStats,
    /// Pacotes perdidos pelos streams já fechados.
    lost_before: u64,
    /// O instante do 1º pacote.
    first_ns: Option<u64>,
}

/// Um erro de escrita: o disco (o motivo provável) ou o reamostrador.
enum WriteError {
    Disk(io::Error),
    Resample(String),
}

impl WriteError {
    fn end(&self, track: Track) -> End {
        match self {
            Self::Disk(e) => {
                eprintln!("gravação: não consegui gravar {}: {e}", track.file());
                End::Disk
            }
            Self::Resample(e) => {
                eprintln!("gravação: reamostragem de {} falhou: {e}", track.file());
                End::Failed
            }
        }
    }
}

fn resampler(rate: u32) -> Result<Resampler, WriteError> {
    Resampler::new(rate, RATE).map_err(|e| WriteError::Resample(e.to_string()))
}

impl TrackWriter {
    fn new(track: Track, consumer: Consumer, wav: Wav, t0_ns: u64) -> Result<Self, WriteError> {
        Ok(Self {
            track,
            resampler: resampler(consumer.format.rate)?,
            stats: TrackStats {
                rate: consumer.format.rate,
                ..TrackStats::default()
            },
            consumer,
            wav: Some(wav),
            written: 0,
            comp: 0,
            raw: Vec::new(),
            mono: Vec::new(),
            out: Vec::new(),
            cycle_sum: 0.0,
            cycle_len: 0,
            level: 0.0,
            loud_at_ns: t0_ns,
            lost_before: 0,
            first_ns: None,
        })
    }

    fn wav(&mut self) -> &mut Wav {
        self.wav.as_mut().expect("aberto até o fim")
    }

    /// Escreve tudo o que está no ring.
    fn drain(&mut self, aligner: &Aligner) -> Result<(), WriteError> {
        let rate = self.consumer.format.rate;
        let channels = self.consumer.format.channels;
        while let Some(packet) = self.consumer.pop(&mut self.raw) {
            align::downmix(&self.raw, channels, &mut self.mono);
            self.stats.received += self.mono.len() as u64;
            let position = self.written + self.resampler.pending();
            let mut skip = 0;
            match aligner.decide(packet.t_ns, position) {
                Action::Silence(frames) => {
                    if self.stats.received == self.mono.len() as u64 {
                        self.stats.startup += frames;
                    } else {
                        self.stats.filled += frames;
                        self.stats.fills += 1;
                        eprintln!(
                            "gravação: {} sem pacotes por {:.1} ms em {:.1} s",
                            self.track.file(),
                            frames as f64 * 1_000.0 / f64::from(RATE),
                            position as f64 / f64::from(RATE)
                        );
                    }
                    self.silence(align::to_rate(frames, rate))?;
                }
                Action::Drop(frames) => {
                    self.stats.dropped += frames;
                    skip = (align::to_rate(frames, rate) as usize).min(self.mono.len());
                }
                Action::Adjust(delta) => self.adjust(delta)?,
            }
            self.out.clear();
            self.resampler
                .push(&self.mono[skip..], &mut self.out)
                .map_err(|e| WriteError::Resample(e.to_string()))?;
            self.write_out()?;
            // O fim do pacote contra o relógio.
            let end_ns =
                packet.t_ns + u64::from(packet.frames) * 1_000_000_000 / u64::from(rate.max(1));
            let error =
                aligner.expected(end_ns) - i128::from(self.written + self.resampler.pending());
            self.stats.max_error = self.stats.max_error.max(error.unsigned_abs() as u64);
            let first = *self.first_ns.get_or_insert(packet.t_ns);
            self.stats.span_ns = end_ns.saturating_sub(first);
        }
        self.stats.lost = self.lost_before + self.consumer.lost();
        Ok(())
    }

    fn adjust(&mut self, delta: i64) -> Result<(), WriteError> {
        if delta == self.comp {
            return Ok(());
        }
        let delta32 = i32::try_from(delta).unwrap_or(if delta < 0 { i32::MIN } else { i32::MAX });
        self.resampler
            .compensate(delta32, (RATE * COMPENSATION_SECS) as i32)
            .map_err(|e| WriteError::Resample(e.to_string()))?;
        self.comp = delta;
        Ok(())
    }

    /// `frames` de silêncio na taxa do dispositivo, pelo reamostrador (a ordem se mantém).
    fn silence(&mut self, frames: u64) -> Result<(), WriteError> {
        let rate = u64::from(self.consumer.format.rate);
        let zeros = vec![0.0; rate as usize];
        let mut left = frames;
        while left > 0 {
            let n = left.min(rate) as usize;
            self.out.clear();
            self.resampler
                .push(&zeros[..n], &mut self.out)
                .map_err(|e| WriteError::Resample(e.to_string()))?;
            self.write_out()?;
            left -= n as u64;
        }
        Ok(())
    }

    /// Grava `out` no WAV.
    fn write_out(&mut self) -> Result<(), WriteError> {
        if self.out.is_empty() {
            return Ok(());
        }
        for &s in &self.out {
            self.cycle_sum += f64::from(s) * f64::from(s);
        }
        self.cycle_len += self.out.len() as u64;
        let len = u32::try_from(self.out.len())
            .map_err(|_| WriteError::Resample("pacote grande demais".into()))?;
        let out = std::mem::take(&mut self.out);
        let wav = self.wav();
        let mut w = wav.get_i16_writer(len);
        for &s in &out {
            w.write_sample(align::to_i16(s));
        }
        let result = w.flush().map_err(hound_io);
        self.out = out;
        result?;
        self.written += u64::from(len);
        Ok(())
    }

    /// O nível desta passada, com a queda lenta do medidor.
    fn end_cycle(&mut self, now_ns: u64) -> f32 {
        let rms = if self.cycle_len == 0 {
            0.0
        } else {
            (self.cycle_sum / self.cycle_len as f64).sqrt() as f32
        };
        if align::db(rms) > align::SILENT_DB {
            self.loud_at_ns = now_ns;
        }
        self.level = align::level(rms).max(self.level * 0.7);
        self.cycle_sum = 0.0;
        self.cycle_len = 0;
        self.level
    }

    fn silent_for(&self, now_ns: u64) -> u64 {
        now_ns.saturating_sub(self.loud_at_ns) / 1_000_000_000
    }

    /// O stream foi reaberto: o que restou do antigo é escrito e o reamostrador passa à taxa
    /// do novo.
    fn swap(&mut self, consumer: Consumer, aligner: &Aligner) -> Result<(), WriteError> {
        self.drain(aligner)?;
        self.out.clear();
        self.resampler
            .flush(&mut self.out)
            .map_err(|e| WriteError::Resample(e.to_string()))?;
        self.write_out()?;
        self.resampler = resampler(consumer.format.rate)?;
        self.comp = 0;
        self.lost_before += self.consumer.lost();
        self.stats.rate = consumer.format.rate;
        self.stats.reopens += 1;
        self.consumer = consumer;
        Ok(())
    }

    /// Esvazia o reamostrador.
    fn flush_resampler(&mut self) -> Result<(), WriteError> {
        self.out.clear();
        self.resampler
            .flush(&mut self.out)
            .map_err(|e| WriteError::Resample(e.to_string()))?;
        self.write_out()
    }

    /// Completa com silêncio até `target` frames e fecha o WAV.
    fn finish(&mut self, target: u64) -> Result<(), WriteError> {
        let missing = target.saturating_sub(self.written);
        if missing > 0 {
            self.out.clear();
            self.out.resize(missing as usize, 0.0);
            self.write_out()?;
        }
        if let Some(wav) = self.wav.take() {
            wav.finalize().map_err(hound_io)?;
        }
        Ok(())
    }
}

fn hound_io(e: hound::Error) -> WriteError {
    match e {
        hound::Error::IoError(e) => WriteError::Disk(e),
        other => WriteError::Disk(io::Error::other(other.to_string())),
    }
}

/// Cria o WAV da trilha.
pub fn create_wav(dir: &Path, track: Track) -> io::Result<Wav> {
    hound::WavWriter::create(dir.join(track.file()), storage::wav_spec()).map_err(|e| match e {
        hound::Error::IoError(e) => e,
        other => io::Error::other(other.to_string()),
    })
}

/// Uma gravação em andamento, do Gravar ao fim.
pub struct Session {
    pub source: Arc<dyn AudioSource>,
    pub choice: Choice,
    pub id: JobId,
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub aligner: Aligner,
    pub shared: Arc<Shared>,
    pub stop: Arc<AtomicU8>,
    pub limits: Limits,
    pub events: Receiver<Event>,
    pub events_tx: Sender<Event>,
    tracks: [TrackWriter; 2],
    streams: [Option<Box<dyn Send>>; 2],
    generation: [u64; 2],
    reopens: Vec<(Track, u64)>,
    last_flush_ns: u64,
    last_disk_ns: u64,
}

/// As duas trilhas já abertas, na ordem de [`Track::ALL`].
pub struct Start {
    pub source: Arc<dyn AudioSource>,
    pub choice: Choice,
    pub id: JobId,
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub t0_ns: u64,
    pub opened: [Opened; 2],
    pub shared: Arc<Shared>,
    pub stop: Arc<AtomicU8>,
    pub limits: Limits,
    pub events: Receiver<Event>,
    pub events_tx: Sender<Event>,
}

impl Session {
    /// Cria os WAVs e prepara a escrita; um erro de disco recusa o Gravar.
    pub fn new(start: Start) -> io::Result<Self> {
        let aligner = Aligner::new(start.t0_ns);
        let [mic, system] = start.opened;
        let make = |track: Track, consumer: Consumer| -> io::Result<TrackWriter> {
            let wav = create_wav(&start.dir, track)?;
            TrackWriter::new(track, consumer, wav, start.t0_ns).map_err(|e| match e {
                WriteError::Disk(e) => e,
                WriteError::Resample(e) => io::Error::other(e),
            })
        };
        let tracks = [
            make(Track::Mic, mic.consumer)?,
            make(Track::System, system.consumer)?,
        ];
        start.shared.set_name(Track::Mic, mic.name);
        start.shared.set_name(Track::System, system.name);
        Ok(Self {
            source: start.source,
            choice: start.choice,
            id: start.id,
            dir: start.dir,
            manifest: start.manifest,
            aligner,
            shared: start.shared,
            stop: start.stop,
            limits: start.limits,
            events: start.events,
            events_tx: start.events_tx,
            tracks,
            streams: [Some(mic.streams), Some(system.streams)],
            generation: [0, 0],
            reopens: Vec::new(),
            last_flush_ns: start.t0_ns,
            last_disk_ns: start.t0_ns,
        })
    }

    /// Grava até o Parar, o encerramento ou um fim automático, e fecha os arquivos.
    pub fn run(mut self) -> Recording {
        let end = loop {
            if let Err(end) = self.cycle() {
                break end;
            }
            match self.stop.load(Ordering::Relaxed) {
                STOP_USER => break End::User,
                STOP_EXIT => break End::Exit,
                _ => thread::sleep(TICK),
            }
        };
        self.finish(end)
    }

    fn cycle(&mut self) -> Result<(), End> {
        for t in &mut self.tracks {
            t.drain(&self.aligner).map_err(|e| e.end(t.track))?;
        }
        while let Ok(event) = self.events.try_recv() {
            self.handle(event)?;
        }
        let now = self.source.now_ns();
        let elapsed_ns = now.saturating_sub(self.aligner.t0_ns());
        self.shared
            .elapsed_ms
            .store(elapsed_ns / 1_000_000, Ordering::Relaxed);
        for t in &mut self.tracks {
            let level = t.end_cycle(now);
            self.shared.set_level(t.track, level);
        }
        let silent = self.tracks[Track::Mic.index()].silent_for(now) >= SILENT_SECS;
        self.shared.mic_silent.store(silent, Ordering::Relaxed);

        if now.saturating_sub(self.last_flush_ns) >= self.limits.flush_every.as_nanos() as u64 {
            self.last_flush_ns = now;
            for t in &mut self.tracks {
                t.wav()
                    .flush()
                    .map_err(hound_io)
                    .map_err(|e| e.end(t.track))?;
            }
        }
        if now.saturating_sub(self.last_disk_ns) >= self.limits.disk_every.as_nanos() as u64 {
            self.last_disk_ns = now;
            if (self.limits.free_space)(&self.dir).is_some_and(|free| free < self.limits.min_free) {
                return Err(End::Disk);
            }
        }
        if elapsed_ns >= self.limits.max.as_nanos() as u64 {
            return Err(End::Limit);
        }
        Ok(())
    }

    fn handle(&mut self, event: Event) -> Result<(), End> {
        let i = event.track.index();
        if event.generation != self.generation[i] {
            return Ok(());
        }
        let lost = match event.track {
            Track::Mic => End::MicLost,
            Track::System => End::OutputLost,
        };
        match event.kind {
            EventKind::Xrun => {
                self.shared.xruns.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            EventKind::Lost => {
                eprintln!("gravação: {} perdido", device_label(event.track));
                Err(lost)
            }
            EventKind::Invalidated => self.reopen(event.track).map_err(|()| lost),
        }
    }

    /// Fecha o stream da trilha e abre de novo no dispositivo escolhido (ou no novo padrão).
    fn reopen(&mut self, track: Track) -> Result<(), ()> {
        let now = self.source.now_ns();
        self.reopens
            .retain(|&(_, at)| now.saturating_sub(at) < REOPEN_WINDOW_NS);
        if self.reopens.iter().filter(|&&(t, _)| t == track).count() as u32 >= MAX_REOPENS {
            eprintln!("gravação: {} reaberto vezes demais", device_label(track));
            return Err(());
        }
        self.reopens.push((track, now));
        let i = track.index();
        self.streams[i] = None;
        self.generation[i] += 1;
        let events = Events::new(track, self.generation[i], self.events_tx.clone());
        let opened = self
            .source
            .open(track, self.choice.device(track), events)
            .map_err(|e| eprintln!("gravação: não reabri {}: {e}", device_label(track)))?;
        eprintln!(
            "gravação: {} reaberto em {}",
            device_label(track),
            opened.name
        );
        self.tracks[i]
            .swap(opened.consumer, &self.aligner)
            .map_err(|e| {
                e.end(track);
            })?;
        self.shared.set_name(track, opened.name);
        self.streams[i] = Some(opened.streams);
        Ok(())
    }

    /// Fecha os streams, escreve o resto, completa as trilhas até o mesmo fim e fecha os
    /// arquivos. Um erro aqui não impede a gravação de voltar: o que está no disco vale.
    fn finish(mut self, end: End) -> Recording {
        self.streams = [None, None];
        let now = self.source.now_ns();
        for t in &mut self.tracks {
            if let Err(e) = t.drain(&self.aligner).and_then(|()| t.flush_resampler()) {
                e.end(t.track);
            }
        }
        let target = self
            .tracks
            .iter()
            .map(|t| t.written)
            .max()
            .unwrap_or(0)
            .max(
                // Um disco cheio não deve crescer as trilhas com silêncio.
                if end == End::Disk {
                    0
                } else {
                    self.aligner.end(now)
                },
            );
        for t in &mut self.tracks {
            if let Err(e) = t.finish(target) {
                e.end(t.track);
            }
        }
        let names = self.shared.names();
        self.manifest.state = RecState::Parada;
        [self.manifest.mic, self.manifest.output] = names.clone();
        if let Err(e) = storage::write_manifest(&self.dir, &self.manifest) {
            eprintln!("gravação: não atualizei o {}: {e}", storage::MANIFEST_FILE);
        }
        let stats = [0, 1].map(|i| self.tracks[i].stats);
        let mut recording = storage::load(&self.dir, end).unwrap_or_else(|| Recording {
            id: self.id,
            dir: self.dir.clone(),
            created: self.manifest.created,
            duration_ms: target * 1_000 / u64::from(RATE),
            size: 0,
            mic: names[0].clone(),
            output: names[1].clone(),
            end,
            stats,
        });
        recording.stats = stats;
        recording
    }
}

fn device_label(track: Track) -> &'static str {
    match track {
        Track::Mic => "microfone",
        Track::System => "saída de som",
    }
}
