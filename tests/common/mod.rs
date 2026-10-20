//! O que os testes de transcrição compartilham: fixtures, modelos e o texto conhecido.

// Cada binário de teste usa uma parte.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use scriba::asr::{self, Model, Models, Source};

pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

pub fn fixture(name: &str) -> PathBuf {
    root().join("tests/fixtures").join(name)
}

pub fn models_dir() -> PathBuf {
    root().join("models")
}

/// Só o base na CPU e o VAD, como no CI (`cargo xtask model --only base,silero`), mesmo
/// que `models/` tenha outros.
pub fn base_models() -> Models {
    let dir = models_dir();
    Models {
        large: None,
        cpu: Some(Model {
            name: "base",
            source: Source::File(dir.join(asr::BASE_FILE)),
        }),
        vad: Some(Source::File(dir.join(asr::VAD_FILE))),
    }
}

/// O áudio de uma fixture em 16 kHz mono.
pub fn audio(name: &str) -> Vec<f32> {
    scriba::media::extract_audio(&fixture(name), |_| {}, &AtomicBool::new(false)).unwrap()
}

/// O texto lido no `fala.wav` (o mesmo de `xtask/src/fixtures.rs`).
pub const FALA: &str = "Bom dia. Esta é uma gravação de teste do Scriba. \
    A reunião sobre o orçamento começa amanhã às nove horas.";

/// As duas frases do `pausas.wav`, com 3 s de silêncio entre elas.
pub const PAUSAS: [&str; 2] = [
    "Primeira parte da reunião, sobre o orçamento.",
    "Segunda parte, depois da pausa, sobre o cronograma.",
];

const KEYWORDS: [&str; 6] = [
    "bom dia",
    "scriba",
    "reunião",
    "orçamento",
    "amanhã",
    "nove horas",
];

/// Quantas palavras-chave do `fala.wav` aparecem no texto (sem caixa).
pub fn keyword_hits(text: &str) -> usize {
    let text = text.to_lowercase();
    KEYWORDS.iter().filter(|k| text.contains(*k)).count()
}

/// O working set do processo agora.
pub fn working_set() -> u64 {
    memory().0
}

/// Working set atual e de pico do processo, em bytes.
pub fn memory() -> (u64, u64) {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: estrutura C sem invariantes; zero é um valor válido.
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: o pseudo-handle do processo atual sempre vale, e `counters` tem o tamanho
    // informado.
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    assert_ne!(ok, 0, "GetProcessMemoryInfo falhou");
    (
        counters.WorkingSetSize as u64,
        counters.PeakWorkingSetSize as u64,
    )
}

/// Amostra o working set a cada 20 ms e guarda o maior: o pico do processo não volta a
/// zero entre uma configuração e outra.
pub struct Sampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: JoinHandle<()>,
}

impl Sampler {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(working_set()));
        let thread = thread::spawn({
            let (stop, peak) = (stop.clone(), peak.clone());
            move || {
                while !stop.load(Ordering::Relaxed) {
                    peak.fetch_max(working_set(), Ordering::Relaxed);
                    thread::sleep(Duration::from_millis(20));
                }
            }
        });
        Self { stop, peak, thread }
    }

    pub fn stop(self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().unwrap();
        self.peak.load(Ordering::Relaxed)
    }
}
