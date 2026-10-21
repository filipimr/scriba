//! Transcrição com o whisper.cpp. Escolhe o backend (GPU dedicada → CPU com o modelo menor),
//! estima o tempo, carrega o modelo na primeira transcrição e o libera após 10 min sem uso,
//! roda o VAD Silero, transcreve em pt, em blocos de ~30 min, e tira as frases-fantasma.

mod backend;
mod blocks;
mod cache;
mod estimate;
mod log;
mod models;
mod phantom;
mod vad;

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::ffi::{c_int, c_void};
use std::fmt;
use std::io;
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use whisper_rs::whisper_rs_sys as sys;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

pub use backend::{Backend, Device, DeviceKind, Plan, devices, free_memory, plans, registry};
pub use blocks::BLOCK;
pub use cache::IdleCache;
pub use estimate::{LOAD_SECS, SHADERS_SECS, estimate, rtf};
pub use models::{BASE_FILE, LARGE_FILE, Model, ModelKind, Models, SMALL_FILE, Source, VAD_FILE};
pub use phantom::is_phantom;

use crate::jobs::{Estimate, Failure};
use crate::transcript::Segment;

/// Tempo sem uso até o modelo sair da memória.
pub const IDLE: Duration = Duration::from_secs(600);

/// Threads do VAD, uma rede pequena: mais que isso só sincroniza.
const VAD_THREADS: i32 = 4;
/// Menos que 0,1 s de fala o whisper nem processa.
const MIN_SPEECH: usize = 1_600;
/// Amostras por segundo, a 16 kHz.
const PER_SEC: usize = 16_000;

/// O áudio 16 kHz mono a transcrever, lido por intervalos: na memória ou em disco (o
/// `media::Spool`), para a transcrição em blocos não segurar a duração inteira.
pub trait Samples {
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Troca o conteúdo de `buf` pelas amostras de `range` (cortado no fim do áudio).
    fn read(&mut self, range: Range<usize>, buf: &mut Vec<f32>) -> io::Result<()>;
}

impl Samples for &[f32] {
    fn len(&self) -> usize {
        <[f32]>::len(self)
    }

    fn read(&mut self, range: Range<usize>, buf: &mut Vec<f32>) -> io::Result<()> {
        let end = range.end.min(<[f32]>::len(self));
        buf.clear();
        buf.extend_from_slice(&self[range.start.min(end)..end]);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Load,
    Failed,
    Canceled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Load => {
                "Não consegui preparar a transcrição. Feche outros programas e tente de novo."
            }
            Self::Failed => "Não consegui transcrever este arquivo. Tente de novo.",
            Self::Canceled => "Cancelado.",
        })
    }
}

impl std::error::Error for Error {}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        match e {
            Error::Canceled => Failure::Canceled,
            other => Failure::Error(other.to_string()),
        }
    }
}

/// O resultado: os segmentos com os tempos do áudio original e quem transcreveu.
#[derive(Clone, Debug, PartialEq)]
pub struct Transcription {
    pub segments: Vec<Segment>,
    /// O candidato que carregou, depois do fallback.
    pub plan: Plan,
    /// `large-v3-turbo`, `small` ou `base`.
    pub model: &'static str,
}

/// O motor de transcrição: os candidatos em ordem e o modelo carregado sob demanda.
pub struct Asr {
    plans: Vec<Plan>,
    models: Models,
    threads: i32,
    /// Beam search no lugar do greedy (benchmark).
    beam: Option<i32>,
    /// Tamanho do bloco, em amostras.
    block: usize,
    cache: IdleCache<Loaded>,
    /// O último candidato que carregou, depois do fallback.
    active: Mutex<Option<Plan>>,
    /// Uma GPU já carregou neste processo: os shaders do Vulkan estão compilados.
    warmed: AtomicBool,
}

impl Asr {
    /// Detecta as GPUs e usa os modelos embutidos ou, em dev, os de `models/`.
    pub fn detect() -> Self {
        let models = Models::locate();
        Self::new(plans(&devices(), &models), models)
    }

    /// Com os candidatos dados, em ordem; `IDLE` sem uso libera o modelo.
    pub fn new(plans: Vec<Plan>, models: Models) -> Self {
        Self::with_idle(plans, models, IDLE)
    }

    pub fn with_idle(plans: Vec<Plan>, models: Models, idle: Duration) -> Self {
        Self {
            plans,
            models,
            threads: default_threads(),
            beam: None,
            block: block_samples(BLOCK),
            cache: IdleCache::new(idle),
            active: Mutex::new(None),
            warmed: AtomicBool::new(false),
        }
    }

    /// Decodifica com beam search de `size` caminhos em vez do greedy: mais lento, só para
    /// comparar a precisão.
    pub fn beam(mut self, size: i32) -> Self {
        self.beam = Some(size);
        self
    }

    /// Transcreve em blocos de `size` em vez de 30 min (os testes usam poucos segundos).
    pub fn blocks(mut self, size: Duration) -> Self {
        self.block = block_samples(size);
        self
    }

    pub fn plans(&self) -> &[Plan] {
        &self.plans
    }

    /// O candidato que vai transcrever: o que carregou por último ou, antes da 1ª carga, o
    /// primeiro da lista. `None` sem nenhum modelo.
    pub fn current_plan(&self) -> Option<Plan> {
        let active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        active.clone().or_else(|| self.plans.first().cloned())
    }

    /// Onde a transcrição roda, para o rodapé.
    pub fn backend_label(&self) -> Option<String> {
        self.current_plan().map(|plan| match plan.backend {
            Backend::Gpu(device) => format!("Transcrição na GPU: {}", device.description),
            Backend::Cpu => "Transcrição no processador".into(),
        })
    }

    /// Quanto deve levar transcrever `audio_ms` de áudio agora; `None` sem nenhum modelo.
    pub fn estimate(&self, audio_ms: u64) -> Option<Estimate> {
        let plan = self.current_plan()?;
        let model = self.models.get(plan.model)?.name;
        Some(estimate(
            &plan,
            model,
            audio_ms,
            self.is_loaded(),
            self.warmed.load(Ordering::Relaxed),
        ))
    }

    /// Se o modelo está na memória (a próxima transcrição não paga a carga).
    pub fn is_loaded(&self) -> bool {
        self.cache.is_loaded()
    }

    /// Carrega o modelo, se preciso, e diz qual candidato ficou.
    pub fn preload(&self) -> Result<Plan, Error> {
        self.cache
            .with(|| self.load(), |loaded| loaded.plan.clone())
    }

    /// Transcreve áudio 16 kHz mono. `progress` recebe de 0 a 100, só quando o valor sobe;
    /// `cancel` é consultado entre os passos do whisper.
    pub fn transcribe(
        &self,
        audio: &[f32],
        progress: impl FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<Transcription, Error> {
        self.transcribe_samples(&mut { audio }, progress, cancel)
    }

    /// Como [`Asr::transcribe`], lendo o áudio um bloco por vez.
    pub fn transcribe_samples(
        &self,
        audio: &mut dyn Samples,
        progress: impl FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<Transcription, Error> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Canceled);
        }
        let mut last = None;
        let mut progress = progress;
        let mut report = move |pct: u8| {
            if last.is_none_or(|last| pct > last) {
                last = Some(pct);
                progress(pct);
            }
        };
        self.cache.with(
            || self.load(),
            |loaded| {
                loaded.run(audio, self.block, &mut report, cancel, &|| {
                    full_params(self.threads, self.beam)
                })
            },
        )?
    }

    fn load(&self) -> Result<Loaded, Error> {
        let (plan, loaded) = load_first(&self.plans, |plan| Loaded::load(plan, &self.models))?;
        eprintln!("asr: transcrevendo com {plan} ({})", loaded.model);
        if matches!(plan.backend, Backend::Gpu(_)) {
            self.warmed.store(true, Ordering::Relaxed);
        }
        *self.active.lock().unwrap_or_else(PoisonError::into_inner) = Some(plan);
        Ok(loaded)
    }
}

/// Um bloco tem pelo menos 1 s: o corte precisa de espaço para achar uma pausa.
fn block_samples(size: Duration) -> usize {
    usize::try_from(size.as_millis())
        .unwrap_or(usize::MAX)
        .saturating_mul(PER_SEC / 1_000)
        .max(PER_SEC)
}

/// Threads do whisper: núcleos lógicos − 2 (etapa 4.1).
fn default_threads() -> i32 {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    cores.saturating_sub(2).max(1) as i32
}

/// Tenta os candidatos em ordem até um carregar. Os erros técnicos vão para o stderr.
fn load_first<T>(
    plans: &[Plan],
    mut load: impl FnMut(&Plan) -> Result<T, String>,
) -> Result<(Plan, T), Error> {
    for plan in plans {
        match load(plan) {
            Ok(loaded) => return Ok((plan.clone(), loaded)),
            Err(e) => eprintln!("asr: {plan} não carregou: {e}"),
        }
    }
    if plans.is_empty() {
        eprintln!("asr: nenhum modelo de transcrição disponível");
    }
    Err(Error::Load)
}

/// O modelo carregado num candidato, com o estado do whisper e o VAD.
struct Loaded {
    plan: Plan,
    model: &'static str,
    /// Mantém o contexto vivo.
    state: WhisperState,
    vad: vad::Vad,
}

impl Loaded {
    fn load(plan: &Plan, models: &Models) -> Result<Self, String> {
        let model = models.get(plan.model).ok_or("modelo ausente")?;
        let vad_model = models.vad.as_ref().ok_or("modelo do VAD ausente")?;
        let mut params = WhisperContextParameters::default();
        params.use_gpu(matches!(plan.backend, Backend::Gpu(_)));
        if let Backend::Gpu(device) = &plan.backend {
            params.gpu_device(device.index);
        }
        params.flash_attn(plan.flash_attn);
        // O estado também entra na conferência: é ele que inicia o backend da GPU.
        let (state, lines) = log::capture(|| {
            let ctx = match &model.source {
                Source::Embedded(bytes) => {
                    WhisperContext::new_from_buffer_with_params(bytes, params)
                }
                Source::File(path) => WhisperContext::new_with_params(path, params),
            }
            .map_err(|e| format!("{} ({}): {e}", model.name, model.source))?;
            ctx.create_state().map_err(|e| e.to_string())
        });
        let state = state?;
        plan.check(&lines)?;
        let bytes = vad_model
            .bytes()
            .map_err(|e| format!("VAD ({vad_model}): {e}"))?;
        Ok(Self {
            plan: plan.clone(),
            model: model.name,
            state,
            vad: vad::Vad::load(&bytes, VAD_THREADS)?,
        })
    }

    /// Transcreve bloco a bloco: cada janela de `block` amostras passa pelo VAD, é cortada
    /// numa pausa perto do fim (`blocks::cut`) e só a fala até o corte vai para o whisper.
    /// O progresso de cada bloco ocupa a fatia dele no áudio inteiro.
    fn run(
        &mut self,
        audio: &mut dyn Samples,
        block: usize,
        report: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
        params: &dyn Fn() -> FullParams<'static, 'static>,
    ) -> Result<Transcription, Error> {
        report(0);
        let total = audio.len();
        let mut segments = Vec::new();
        let mut window = Vec::new();
        let mut cursor = 0;
        while cursor < total {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Canceled);
            }
            audio
                .read(cursor..cursor.saturating_add(block), &mut window)
                .map_err(|e| {
                    eprintln!("asr: não consegui ler o áudio extraído: {e}");
                    Error::Failed
                })?;
            if window.is_empty() {
                break;
            }
            let mut spans = self.vad.speech(&window).map_err(|e| {
                eprintln!("asr: {e}");
                Error::Failed
            })?;
            let len = if cursor + window.len() >= total {
                window.len()
            } else {
                blocks::cut(&spans, window.len()).unwrap_or_else(|| {
                    eprintln!(
                        "asr: sem pausa perto de {} s; o bloco corta no fim da janela",
                        (cursor + window.len()) / PER_SEC
                    );
                    window.len()
                })
            };
            spans.retain(|s| s.start < len);
            if let Some(last) = spans.last_mut() {
                last.end = last.end.min(len);
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Canceled);
            }
            let (speech, map) = vad::splice(&window[..len], &spans);
            let map = map.offset(cursor);
            let (base, share) = (cursor as f64 / total as f64, len as f64 / total as f64);
            let mut block_report =
                |pct: u8| report(((base + share * f64::from(pct) / 100.0) * 100.0) as u8);
            self.whisper(
                &speech,
                &map,
                &mut block_report,
                cancel,
                params(),
                &mut segments,
            )?;
            cursor += len;
        }
        report(100);
        Ok(Transcription {
            segments,
            plan: self.plan.clone(),
            model: self.model,
        })
    }

    /// Roda o whisper na fala de um bloco e acrescenta a `segments` o que ele achou, com os
    /// tempos do original.
    fn whisper(
        &mut self,
        speech: &[f32],
        map: &vad::TimeMap,
        report: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
        mut params: FullParams<'static, 'static>,
        segments: &mut Vec<Segment>,
    ) -> Result<(), Error> {
        if speech.len() < MIN_SPEECH {
            return Ok(());
        }
        let hooks = Hooks {
            report: RefCell::new(report),
            cancel,
            panic: Cell::new(None),
        };
        // SAFETY: os callbacks recebem `&hooks`, que vive até o fim do `full`; o
        // whisper.cpp só os chama durante a chamada, na thread dela.
        unsafe {
            params.set_progress_callback(Some(on_progress));
            params.set_progress_callback_user_data((&raw const hooks).cast_mut().cast());
            params.set_abort_callback(Some(on_abort));
            params.set_abort_callback_user_data((&raw const hooks).cast_mut().cast());
        }
        let result = self.state.full(params, speech);
        if let Some(payload) = hooks.panic.take() {
            panic::resume_unwind(payload);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Canceled);
        }
        result.map_err(|e| {
            eprintln!("asr: o whisper falhou: {e}");
            Error::Failed
        })?;
        for segment in self.state.as_iter() {
            let text = segment
                .to_str_lossy()
                .map(|t| t.trim().to_string())
                .unwrap_or_default();
            if is_phantom(&text, segment.no_speech_probability()) {
                continue;
            }
            let (start_ms, end_ms) =
                map.segment_ms(segment.start_timestamp(), segment.end_timestamp());
            segments.push(Segment::new(start_ms, end_ms, text));
        }
        Ok(())
    }
}

/// Os parâmetros do app: pt, sem o texto anterior como contexto (evita repetições em
/// cadeia), sem os tokens de não fala ("[MÚSICA]") e greedy com os 5 candidatos do
/// whisper.cpp quando a temperatura sobe.
fn full_params(threads: i32, beam: Option<i32>) -> FullParams<'static, 'static> {
    let mut params = FullParams::new(match beam {
        Some(beam_size) => SamplingStrategy::BeamSearch {
            beam_size,
            patience: -1.0,
        },
        None => SamplingStrategy::Greedy { best_of: 5 },
    });
    params.set_language(Some("pt"));
    params.set_n_threads(threads);
    params.set_no_context(true);
    params.set_suppress_nst(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params
}

/// O que os callbacks do whisper.cpp alcançam. Os "safe" do whisper-rs 0.16 vazam a closure
/// a cada chamada, e o de abort registra o trampolim com o tipo errado.
struct Hooks<'a> {
    report: RefCell<&'a mut dyn FnMut(u8)>,
    cancel: &'a AtomicBool,
    /// Um pânico no progresso não pode atravessar o C: fica guardado, o whisper é
    /// interrompido e o pânico continua depois do `full`.
    panic: Cell<Option<Box<dyn Any + Send>>>,
}

unsafe extern "C" fn on_progress(
    _: *mut sys::whisper_context,
    _: *mut sys::whisper_state,
    pct: c_int,
    data: *mut c_void,
) {
    // SAFETY: `data` aponta para os `Hooks` de `Loaded::run`, vivos durante o `full`.
    let hooks = unsafe { &*data.cast::<Hooks>() };
    let panicked = hooks.panic.take();
    if panicked.is_none() {
        let pct = pct.clamp(0, 99) as u8;
        let result = panic::catch_unwind(AssertUnwindSafe(|| (hooks.report.borrow_mut())(pct)));
        hooks.panic.set(result.err());
    } else {
        hooks.panic.set(panicked);
    }
}

unsafe extern "C" fn on_abort(data: *mut c_void) -> bool {
    // SAFETY: como em `on_progress`.
    let hooks = unsafe { &*data.cast::<Hooks>() };
    let panicked = hooks.panic.take();
    let abort = panicked.is_some() || hooks.cancel.load(Ordering::Relaxed);
    hooks.panic.set(panicked);
    abort
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primeiro_candidato_que_carrega() {
        let plans = [
            Plan {
                flash_attn: true,
                ..Plan::cpu()
            },
            Plan::cpu(),
        ];
        let mut tried = 0;
        let (plan, loaded) = load_first(&plans, |p| {
            tried += 1;
            if p.flash_attn {
                Err("sem memória".into())
            } else {
                Ok("carregado")
            }
        })
        .unwrap();
        assert_eq!((plan, loaded, tried), (Plan::cpu(), "carregado", 2));
    }

    #[test]
    fn todos_falham_da_erro_amigavel() {
        let plans = [Plan::cpu(), Plan::cpu()];
        let err = load_first(&plans, |_| Err::<(), _>("falhou".into())).unwrap_err();
        assert_eq!(err, Error::Load);
        assert_eq!(
            Failure::from(err),
            Failure::Error(
                "Não consegui preparar a transcrição. Feche outros programas e tente de novo."
                    .into()
            )
        );
        assert_eq!(load_first(&[], |_| Ok(())).unwrap_err(), Error::Load);
    }

    #[test]
    fn erros_viram_falha_do_job() {
        assert_eq!(Failure::from(Error::Canceled), Failure::Canceled);
        assert_eq!(
            Failure::from(Error::Failed),
            Failure::Error("Não consegui transcrever este arquivo. Tente de novo.".into())
        );
    }

    fn models() -> Models {
        let model = |name| Model {
            name,
            source: Source::Embedded(b""),
        };
        Models {
            large: Some(model("large-v3-turbo")),
            cpu: Some(model("small")),
            vad: Some(Source::Embedded(b"")),
        }
    }

    #[test]
    fn rodape_e_estimativa_seguem_o_primeiro_candidato() {
        let rtx = Plan {
            backend: Backend::Gpu(Device {
                index: 1,
                name: "Vulkan1".into(),
                description: "NVIDIA GeForce RTX 3050 6GB Laptop GPU".into(),
                kind: DeviceKind::Dedicated,
                memory: 6 << 30,
            }),
            model: ModelKind::Large,
            flash_attn: true,
        };
        let asr = Asr::new(vec![rtx.clone(), Plan::cpu()], models());
        assert_eq!(asr.current_plan(), Some(rtx));
        assert_eq!(
            asr.backend_label().as_deref(),
            Some("Transcrição na GPU: NVIDIA GeForce RTX 3050 6GB Laptop GPU")
        );
        // 60 s × 0,032 + carga + shaders.
        assert_eq!(
            asr.estimate(60_000),
            Some(Estimate {
                secs: 13,
                gpu: true
            })
        );

        let cpu = Asr::new(vec![Plan::cpu()], models());
        assert_eq!(
            cpu.backend_label().as_deref(),
            Some("Transcrição no processador")
        );
        assert_eq!(
            cpu.estimate(600_000),
            Some(Estimate {
                secs: 77,
                gpu: false
            })
        );
    }

    #[test]
    fn sem_candidato_nao_ha_rodape_nem_estimativa() {
        let asr = Asr::new(Vec::new(), Models::default());
        assert_eq!(asr.current_plan(), None);
        assert_eq!(asr.backend_label(), None);
        assert_eq!(asr.estimate(60_000), None);
    }

    #[test]
    fn threads_deixam_dois_nucleos_livres() {
        let cores = std::thread::available_parallelism().unwrap().get() as i32;
        assert_eq!(default_threads(), (cores - 2).max(1));
    }
}
