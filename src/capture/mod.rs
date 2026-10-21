//! A gravação ao vivo: o microfone ("Você") e o som do PC ("Outros", o loopback da saída com
//! silêncio tocando nela para o relógio nunca parar) em duas trilhas WAV de 32 kHz, alinhadas
//! pelo instante de captura de cada pacote. O [`Recorder`] grava uma por vez e entrega cada
//! gravação terminada ao `on_finish` (o app a põe na fila).
//!
//! A fonte é o trait [`AudioSource`]: o cpal no app ([`CpalSource`]) e o [`FakeSource`] nos
//! testes e no `--fake-audio`.

pub mod align;
#[cfg(any(test, feature = "test-support"))]
pub mod fake;
mod source;
pub mod storage;
mod wasapi;
mod writer;

use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(any(test, feature = "test-support"))]
pub use fake::{FakeSource, ScriptedSource};
pub use source::{
    AudioSource, Choice, Consumer, Device, Devices, Error, Event, EventKind, Events, Format,
    Opened, Packet, Producer, RING_SECS, Track, ring,
};
pub use storage::{End, Recording, TrackStats, recover};
pub use wasapi::{CpalSource, is_bluetooth};
pub use writer::SILENT_SECS;

use crate::clock::LocalTime;
use crate::jobs::JobId;
use storage::{Manifest, RecState};
use writer::{RUN, STOP_EXIT, STOP_USER, Session, Shared, Start};

/// Limites da gravação. O padrão é o do app; os testes encurtam.
#[derive(Clone)]
pub struct Limits {
    /// O teto de duração.
    pub max: Duration,
    /// Livre no disco para começar.
    pub min_free_start: u64,
    /// Abaixo disso, a gravação para (a transcrição ainda precisa de espaço).
    pub min_free: u64,
    pub disk_every: Duration,
    pub flush_every: Duration,
    /// Bytes livres no volume da pasta.
    pub free_space: FreeSpace,
}

/// Quantos bytes estão livres no volume de um caminho; `None` se o Windows não informa.
pub type FreeSpace = Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

impl Default for Limits {
    fn default() -> Self {
        Self {
            max: Duration::from_secs(12 * 3_600),
            min_free_start: 1 << 30,
            min_free: 512 << 20,
            disk_every: Duration::from_secs(10),
            flush_every: Duration::from_secs(5),
            free_space: Arc::new(storage::free_space),
        }
    }
}

/// O que a tela mostra.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle {
        /// A última gravação que terminou nesta execução e por quê.
        last: Option<(JobId, End)>,
    },
    Recording {
        id: JobId,
        created: LocalTime,
        elapsed_ms: u64,
        /// "Você" e "Outros", de 0 a 1.
        levels: [f32; 2],
        /// O microfone está mudo há [`SILENT_SECS`].
        mic_silent: bool,
        /// O computador suspendeu no meio: o tempo suspenso ficou de fora da gravação.
        suspended: bool,
        mic: String,
        output: String,
    },
}

type OnFinish = Box<dyn Fn(Recording) + Send + Sync>;

/// Handle clonável da gravação.
#[derive(Clone)]
pub struct Recorder(Arc<Inner>);

struct Inner {
    source: Arc<dyn AudioSource>,
    root: PathBuf,
    limits: Limits,
    state: Mutex<State>,
    on_finish: Mutex<Option<OnFinish>>,
}

enum State {
    Idle { last: Option<(JobId, End)> },
    Recording(Active),
}

struct Active {
    id: JobId,
    created: LocalTime,
    shared: Arc<Shared>,
    stop: Arc<AtomicU8>,
    /// Sai daqui quando alguém espera o fim (Parar ou encerramento).
    thread: Option<JoinHandle<Recording>>,
}

impl Recorder {
    /// As gravações ficam em `root` (`<data-dir>/gravacoes`).
    pub fn new(source: Arc<dyn AudioSource>, root: PathBuf, limits: Limits) -> Self {
        Self(Arc::new(Inner {
            source,
            root,
            limits,
            state: Mutex::new(State::Idle { last: None }),
            on_finish: Mutex::new(None),
        }))
    }

    pub fn root(&self) -> &Path {
        &self.0.root
    }

    /// Quem recebe cada gravação terminada (menos a do encerramento, que volta na próxima
    /// abertura). Chamado antes de o status voltar a "parado".
    pub fn on_finish(&self, f: impl Fn(Recording) + Send + Sync + 'static) {
        *self
            .0
            .on_finish
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(f));
    }

    pub fn devices(&self) -> Result<Devices, Error> {
        self.0.source.devices()
    }

    pub fn status(&self) -> Status {
        match &*self.state() {
            State::Idle { last } => Status::Idle { last: *last },
            State::Recording(active) => {
                let [mic, output] = active.shared.names();
                Status::Recording {
                    id: active.id,
                    created: active.created,
                    elapsed_ms: active.shared.elapsed_ms.load(Ordering::Relaxed),
                    levels: active.shared.levels(),
                    mic_silent: active.shared.mic_silent.load(Ordering::Relaxed),
                    suspended: active.shared.suspended.load(Ordering::Relaxed),
                    mic,
                    output,
                }
            }
        }
    }

    /// Grava nos dispositivos escolhidos. Uma gravação por vez.
    pub fn start(&self, choice: &Choice) -> Result<Status, Error> {
        let mut state = self.state();
        if matches!(*state, State::Recording(_)) {
            return Err(Error::Busy);
        }
        let limits = self.0.limits.clone();
        let _ = std::fs::create_dir_all(&self.0.root);
        if (limits.free_space)(&self.0.root).is_some_and(|free| free < limits.min_free_start) {
            return Err(Error::Disk);
        }
        let id = JobId::generate();
        let created = LocalTime::now();
        let dir = storage::create(&self.0.root, id).map_err(|e| {
            eprintln!(
                "gravação: não criei a pasta em {}: {e}",
                self.0.root.display()
            );
            Error::Files
        })?;
        let fail = |e: Error| {
            storage::remove_dir_quietly(&dir);
            e
        };
        // O instante do Gravar é a posição 0 das duas trilhas.
        let t0_ns = self.0.source.now_ns();
        let (events_tx, events) = mpsc::channel();
        let open = |track: Track| {
            self.0.source.open(
                track,
                choice.device(track),
                Events::new(track, 0, events_tx.clone()),
            )
        };
        let mic = open(Track::Mic).map_err(fail)?;
        let system = open(Track::System).map_err(fail)?;
        let manifest = Manifest {
            version: storage::MANIFEST_VERSION,
            id: id.to_string(),
            created,
            mic: mic.name.clone(),
            output: system.name.clone(),
            state: RecState::Gravando,
        };
        storage::write_manifest(&dir, &manifest).map_err(|e| {
            eprintln!("gravação: não gravei o manifesto: {e}");
            fail(Error::Files)
        })?;
        let shared = Arc::new(Shared::default());
        let stop = Arc::new(AtomicU8::new(RUN));
        let session = Session::new(Start {
            source: self.0.source.clone(),
            choice: choice.clone(),
            id,
            dir: dir.clone(),
            manifest,
            t0_ns,
            opened: [mic, system],
            shared: shared.clone(),
            stop: stop.clone(),
            limits,
            events,
            events_tx,
        })
        .map_err(|e| {
            eprintln!("gravação: não criei os WAVs: {e}");
            fail(Error::Files)
        })?;
        let recorder = self.clone();
        let thread = thread::Builder::new()
            .name("scriba-gravacao".into())
            .spawn(move || recorder.record(session, &dir))
            .map_err(|e| {
                eprintln!("gravação: não criei a thread: {e}");
                Error::Files
            })?;
        *state = State::Recording(Active {
            id,
            created,
            shared,
            stop,
            thread: Some(thread),
        });
        drop(state);
        Ok(self.status())
    }

    /// Para a gravação e espera os arquivos fecharem. A gravação já foi entregue ao
    /// `on_finish` quando isto volta.
    pub fn stop(&self) -> Result<Recording, Error> {
        self.end(STOP_USER).ok_or(Error::NotRecording)
    }

    /// Encerramento do app (Ctrl+C): fecha os WAVs sem criar o job; a gravação volta na
    /// próxima abertura.
    pub fn finish_on_exit(&self) -> Option<Recording> {
        self.end(STOP_EXIT)
    }

    fn end(&self, how: u8) -> Option<Recording> {
        let thread = {
            let mut state = self.state();
            let State::Recording(active) = &mut *state else {
                return None;
            };
            active.stop.store(how, Ordering::Relaxed);
            active.thread.take()?
        };
        thread.join().ok()
    }

    /// A thread da gravação. Um panic (bug) não perde o que já está no disco.
    fn record(&self, session: Session, dir: &Path) -> Recording {
        let id = session.id;
        let recording =
            panic::catch_unwind(AssertUnwindSafe(|| session.run())).unwrap_or_else(|_| {
                eprintln!("gravação: a thread de escrita entrou em pânico");
                storage::load(dir, End::Failed).unwrap_or(Recording {
                    id,
                    dir: dir.to_path_buf(),
                    created: LocalTime::now(),
                    duration_ms: 0,
                    size: 0,
                    mic: String::new(),
                    output: String::new(),
                    end: End::Failed,
                    stats: Default::default(),
                })
            });
        if recording.end != End::Exit
            && let Some(f) = &*self
                .0
                .on_finish
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        {
            f(recording.clone());
        }
        let mut state = self.state();
        if matches!(&*state, State::Recording(a) if a.id == id) {
            *state = State::Idle {
                last: Some((id, recording.end)),
            };
        }
        recording
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
