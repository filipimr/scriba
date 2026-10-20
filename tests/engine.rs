//! O pipeline do app de ponta a ponta: a fila real com o `Engine` (FFmpeg + whisper com o
//! modelo base na CPU, como no CI) e as fixtures. O de hardware usa a GPU dedicada:
//! `cargo test --release --features vulkan --test engine -- --ignored`.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use common::{base_models, fixture, keyword_hits};
use scriba::asr::{Asr, Plan};
use scriba::clock::LocalTime;
use scriba::engine::Engine;
use scriba::jobs::{Ctx, Failure, Job, JobId, Jobs, Media, Outputs, Pipeline, Stage, State};
use scriba::transcript::{self, MANIFEST_VERSION, Manifest, MediaKind};

fn engine(out: &Path) -> Engine {
    Engine::with_asr(out.into(), Asr::new(vec![Plan::cpu()], base_models()))
}

/// Uma hora fixa, para o nome da pasta ser previsível.
fn created() -> LocalTime {
    LocalTime {
        year: 2026,
        month: 10,
        day: 20,
        hour: 9,
        minute: 15,
        second: 0,
        offset_minutes: -180,
    }
}

/// Copia a fixture para `uploads` (o worker apaga o envio) e monta o job com o probe real.
fn job(jobs: &Jobs, name: &str) -> Job {
    let id = JobId::generate();
    let ext = Path::new(name).extension().unwrap().to_str().unwrap();
    let input = jobs.upload_path(id, ext);
    std::fs::copy(fixture(name), &input).unwrap();
    let probe = jobs.probe(name, &input).unwrap();
    Job {
        id,
        name: name.into(),
        media: probe.media,
        size: std::fs::metadata(&input).unwrap().len(),
        duration_ms: probe.duration_ms,
        created: created(),
        estimate: probe.estimate,
        input,
        state: State::Queued,
    }
}

fn wait_done(jobs: &Jobs, id: JobId) -> Outputs {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let job = jobs.get(id).unwrap();
        match job.state {
            State::Done { outputs } => return outputs,
            ref s if s.is_terminal() => panic!("{} terminou em {s:?}", job.name),
            _ => {}
        }
        assert!(Instant::now() < deadline, "{}: {:?}", job.name, job.state);
        thread::sleep(Duration::from_millis(20));
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    out: PathBuf,
    jobs: Jobs,
    worker: Option<scriba::jobs::Worker>,
}

impl Setup {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("saidas");
        let (jobs, worker) =
            Jobs::start(dir.path().join("uploads"), Arc::new(engine(&out))).unwrap();
        Self {
            _dir: dir,
            out,
            jobs,
            worker: Some(worker),
        }
    }

    fn run(&self, name: &str) -> Outputs {
        let job = job(&self.jobs, name);
        let id = job.id;
        self.jobs.submit(job);
        wait_done(&self.jobs, id)
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            assert!(worker.shutdown(Duration::from_secs(30)));
        }
    }
}

#[test]
fn video_vira_txt_md_e_manifesto() {
    let setup = Setup::new();
    let outputs = setup.run("fala.mp4");
    assert_eq!(
        outputs.dir,
        setup.out.join("2026-10-20 0915 - fala"),
        "pasta com a hora do envio"
    );

    let txt = std::fs::read_to_string(&outputs.txt).unwrap();
    let parsed = transcript::parse(&txt);
    assert_eq!(parsed.file.as_deref(), Some("fala.mp4"));
    assert_eq!(parsed.date.as_deref(), Some("20/10/2026 09:15"));
    // Duração do contêiner: o vídeo do fala.mp4 passa do áudio (13,7 s).
    assert_eq!(parsed.duration.as_deref(), Some("00:00:13"));
    assert!(keyword_hits(&txt) >= 4, "{txt}");
    let md = std::fs::read_to_string(&outputs.md).unwrap();
    assert!(md.starts_with("# fala.mp4\n"), "{md}");
    assert!(keyword_hits(&md) >= 4, "{md}");

    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(outputs.dir.join(transcript::JOB_FILE)).unwrap())
            .unwrap();
    assert_eq!(manifest.version, MANIFEST_VERSION);
    assert_eq!(manifest.name, "fala.mp4");
    assert_eq!(manifest.media, MediaKind::Video);
    assert_eq!(manifest.created, "2026-10-20T09:15:00-03:00");
    assert_eq!(
        (manifest.backend.as_str(), manifest.model.as_str()),
        ("CPU", "base")
    );
    assert!(manifest.duration_ms > 13_000, "{manifest:?}");
    let t = &manifest.timings;
    assert!(
        t.transcribe_ms > 0 && t.total_ms >= t.extract_ms + t.transcribe_ms,
        "{t:?}"
    );
}

#[test]
fn pausa_longa_da_dois_paragrafos_e_nome_repetido_ganha_sufixo() {
    let setup = Setup::new();
    let first = setup.run("pausas.wav");
    let parsed = transcript::parse(&std::fs::read_to_string(&first.txt).unwrap());
    assert_eq!(parsed.blocks.len(), 2, "{parsed:?}");
    assert!(parsed.blocks[1].start_ms.unwrap() >= 5_000, "{parsed:?}");

    // Mesmo nome e mesma hora: a segunda pasta ganha " (2)".
    let second = setup.run("pausas.wav");
    assert_eq!(first.dir, setup.out.join("2026-10-20 0915 - pausas"));
    assert_eq!(second.dir, setup.out.join("2026-10-20 0915 - pausas (2)"));
}

#[test]
fn cancelar_na_transcricao_nao_deixa_pasta() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("saidas");
    let engine = engine(&out);
    let input = dir.path().join("entrada.wav");
    std::fs::copy(fixture("fala.wav"), &input).unwrap();
    let job = Job {
        id: JobId::generate(),
        name: "fala.wav".into(),
        media: Media::Audio,
        size: 1,
        duration_ms: 10_931,
        created: created(),
        estimate: None,
        input,
        state: State::Queued,
    };
    // Cancela no primeiro progresso do whisper, depois do VAD.
    let cancel = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let ctx = Ctx::new(cancel.clone(), {
        let seen = seen.clone();
        move |stage, pct| {
            seen.lock().unwrap().push((stage, pct));
            if stage == Stage::Transcribing && pct > 0 {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });
    assert_eq!(engine.run(&job, &ctx), Err(Failure::Canceled));
    let seen = seen.lock().unwrap();
    assert!(seen.contains(&(Stage::ExtractingAudio, 100)), "{seen:?}");
    assert!(
        seen.iter()
            .any(|&(s, p)| s == Stage::Transcribing && p < 100),
        "{seen:?}"
    );
    assert!(!out.exists() || std::fs::read_dir(&out).unwrap().count() == 0);
}

#[test]
fn probe_das_fixtures() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    for (name, media) in [
        ("fala.mp4", Media::Video),
        ("fala.mkv", Media::Video),
        ("fala.mp3", Media::Audio),
        ("fala.m4a", Media::Audio),
        ("fala.ogg", Media::Audio),
        ("fala.wav", Media::Audio),
    ] {
        let probe = engine.probe(name, &fixture(name)).unwrap();
        assert_eq!(probe.media, media, "{name}");
        assert!(probe.duration_ms > 10_000, "{name}: {probe:?}");
        // Base na CPU, modelo ainda não carregado: RTF + carga, abaixo de 1 min.
        let estimate = probe.estimate.unwrap();
        assert!(
            !estimate.gpu && (1..60).contains(&estimate.secs),
            "{estimate:?}"
        );
    }
    assert_eq!(
        engine.probe("tela.mp4", &fixture("video-sem-audio.mp4")),
        Err(Failure::Error(
            "Este arquivo não tem áudio. Escolha um vídeo com som.".into()
        ))
    );
    let junk = dir.path().join("nota.wav");
    std::fs::write(&junk, b"isto nao e audio").unwrap();
    assert_eq!(
        engine.probe("nota.wav", &junk),
        Err(Failure::Error(
            "Não reconheci este arquivo como vídeo ou áudio.".into()
        ))
    );
    assert_eq!(
        engine.backend().as_deref(),
        Some("Transcrição no processador")
    );
}

/// O app de verdade com a GPU: detecção, rodapé, estimativa na GPU e o job concluído.
#[test]
#[ignore = "hardware"]
fn na_gpu_dedicada() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("saidas");
    let engine = Engine::new(out.clone());
    let backend = engine.backend().unwrap();
    println!("{backend}");
    assert!(backend.starts_with("Transcrição na GPU: "), "{backend}");
    let probe = engine.probe("fala.mp4", &fixture("fala.mp4")).unwrap();
    println!("{probe:?}");
    assert!(probe.estimate.unwrap().gpu);

    let (jobs, worker) = Jobs::start(dir.path().join("uploads"), Arc::new(engine)).unwrap();
    let job = job(&jobs, "fala.mp4");
    let id = job.id;
    jobs.submit(job);
    let outputs = wait_done(&jobs, id);
    let txt = std::fs::read_to_string(&outputs.txt).unwrap();
    println!("{txt}");
    assert!(keyword_hits(&txt) >= 4, "{txt}");
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(outputs.dir.join(transcript::JOB_FILE)).unwrap())
            .unwrap();
    assert_eq!(manifest.model, "large-v3-turbo");
    assert!(worker.shutdown(Duration::from_secs(30)));
}
