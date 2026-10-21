//! O pipeline do app de ponta a ponta: a fila real com o `Engine` (FFmpeg + whisper com o
//! modelo base na CPU, como no CI) e as fixtures. Os de hardware usam a GPU dedicada e
//! áudios longos de verdade, um teste por vez, porque o pico de RAM é o do processo:
//!
//! `cargo test --release --features vulkan --test engine -- --ignored --test-threads=1 --nocapture`
//!
//! - `SCRIBA_LONG_AUDIO` (`uma_hora`) e `SCRIBA_LONG_AUDIO_3H` (`tres_horas`): os áudios ou
//!   vídeos longos.
//! - `SCRIBA_LONG_DEVICES`: `gpu` (a dedicada) e `cpu` (o modelo da CPU), separados por
//!   vírgula; padrão: os dois.
//! - `SCRIBA_LONG_BLOCK_MIN`: o tamanho do bloco em minutos, no lugar de 30 (um valor
//!   enorme mede "sem blocos").
//!
//! As transcrições longas ficam em `%TEMP%\scriba-longo`, para conferir o texto.
//!
//! `gravacao_real` transcreve as trilhas reais da etapa 6.1 com a "Você" da Maria misturada
//! no microfone (deixa o `.txt` em `%TEMP%\scriba-gravacao\transcricoes`).

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use common::{Sampler, base_models, fixture, keyword_hits, working_set};
use scriba::asr::{self, Asr, Backend, Models, Plan};
use scriba::clock::LocalTime;
use scriba::engine::Engine;
use scriba::jobs::{Audio, Ctx, Failure, Job, JobId, Jobs, Media, Outputs, Pipeline, Stage, State};
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
            State::Done { outputs, .. } => return outputs,
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
        Self::with(engine)
    }

    fn with(engine: impl FnOnce(&Path) -> Engine) -> Self {
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

/// "Guardar áudio" com o `Engine`: o vídeo vira um `.m4a` só com o som, com a duração do
/// áudio, e o envio sai de `uploads/`.
#[test]
fn guardar_audio_do_video_concluido() {
    let setup = Setup::new();
    let job = job(&setup.jobs, "fala.mp4");
    let (id, input) = (job.id, job.input.clone());
    setup.jobs.submit(job);
    let outputs = wait_done(&setup.jobs, id);
    assert!(input.exists(), "o envio espera a resposta");

    let chosen = outputs.dir.join("fala.m4a");
    let answer = chosen.clone();
    setup
        .jobs
        .save_audio(id, move |_, _| Ok(Some(answer)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match setup.jobs.get(id).unwrap().state {
            State::Done {
                audio: Audio::Saved { path },
                ..
            } => {
                assert_eq!(path, chosen);
                break;
            }
            State::Done { audio, .. } if !audio.busy() => panic!("não guardou: {audio:?}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "o áudio não terminou");
        thread::sleep(Duration::from_millis(20));
    }
    let probe = scriba::media::probe(&chosen).unwrap();
    assert!(probe.has_audio && !probe.has_video, "{probe:?}");
    assert!(
        (probe.duration_ms as i64 - 10_931).abs() <= 100,
        "{probe:?}"
    );
    assert!(!input.exists(), "guardado apaga o envio");
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
fn blocos_pelo_audio_temporario() {
    // Blocos de 5 s nos ~13 s do `pausas.wav`: o mesmo texto em dois parágrafos.
    let setup = Setup::with(|out| {
        Engine::with_asr(
            out.into(),
            Asr::new(vec![Plan::cpu()], base_models()).blocks(Duration::from_secs(5)),
        )
    });
    let outputs = setup.run("pausas.wav");
    let parsed = transcript::parse(&std::fs::read_to_string(&outputs.txt).unwrap());
    assert_eq!(parsed.blocks.len(), 2, "{parsed:?}");
    let text = format!("{:?}", parsed.blocks).to_lowercase();
    assert!(
        text.contains("orçamento") && text.contains("cronograma"),
        "{text}"
    );
    // O áudio extraído (`<id>.f32`) não fica em `uploads`; o envio fica até a resposta a
    // "Guardar o áudio?".
    let left: Vec<String> = std::fs::read_dir(setup.jobs.uploads_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(left.len() == 1 && left[0].ends_with(".wav"), "{left:?}");
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
    assert!(!scriba::engine::spool_path(&job.input).exists());
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

/// As falas das fixtures `voce.wav`/`outros.wav` (a `CONVERSA` do `xtask fixtures`).
const VOCE: [&str; 2] = [
    "Obrigado. Eu preparei a planilha com os números do trimestre.",
    "Combinado. Eu envio o relatório por email amanhã cedo.",
];
const OUTROS: [&str; 2] = [
    "Bom dia a todos. Vamos começar pela revisão do orçamento anual.",
    "Ótimo. Depois falamos do cronograma de entregas do projeto.",
];
/// Palavras que só um dos lados fala (sem acento, como em `merge::words`).
const VOCE_WORDS: [&str; 4] = ["planilha", "trimestre", "relatorio", "email"];
const OUTROS_WORDS: [&str; 4] = ["orcamento", "revisao", "cronograma", "entregas"];

/// Uma gravação em `root` com as trilhas dadas, como o `Recorder` deixa.
fn recording_from(root: &Path, mic: &Path, system: &Path) -> scriba::capture::Recording {
    use scriba::capture::{End, Track, storage};
    let id = JobId::generate();
    let dir = storage::create(root, id).unwrap();
    std::fs::copy(mic, dir.join(Track::Mic.file())).unwrap();
    std::fs::copy(system, dir.join(Track::System.file())).unwrap();
    storage::write_manifest(
        &dir,
        &storage::Manifest {
            version: storage::MANIFEST_VERSION,
            id: id.to_string(),
            created: created(),
            mic: "Microfone".into(),
            output: "Alto-falantes".into(),
            state: storage::RecState::Parada,
        },
    )
    .unwrap();
    storage::load(&dir, End::User).unwrap()
}

/// O texto de cada falante no `.txt`, em palavras normalizadas.
fn speaker_blocks(txt: &str, speaker: transcript::Speaker) -> Vec<String> {
    transcript::parse(txt)
        .blocks
        .into_iter()
        .filter(|b| b.speaker == Some(speaker))
        .map(|b| transcript::merge::words(&b.text).join(" "))
        .collect()
}

fn hits(blocks: &[String], words: &[&str]) -> usize {
    let all: Vec<&str> = blocks.iter().flat_map(|b| b.split(' ')).collect();
    words.iter().filter(|w| all.contains(w)).count()
}

/// Uma gravação com eco (o microfone ouviu "Outros" a −12 dB, 60 ms depois): o texto sai
/// rotulado, sem as falas de "Outros" repetidas como "Você", e o "Guardar áudio" mistura as
/// duas trilhas e apaga a pasta da gravação.
#[test]
fn gravacao_sai_rotulada_e_sem_eco() {
    let setup = Setup::new();
    let root = setup.out.with_file_name("gravacoes");
    let rec = recording_from(&root, &fixture("voce-com-eco.wav"), &fixture("outros.wav"));
    let dir = rec.dir.clone();
    let id = setup.jobs.submit_recording(rec);
    let outputs = wait_done(&setup.jobs, id);
    assert_eq!(outputs.dir, setup.out.join("2026-10-20 0915 - Gravação"));

    let txt = std::fs::read_to_string(&outputs.txt).unwrap();
    println!("{txt}");
    // O microfone sozinho tem o eco transcrito: sem isso, o teste não provaria a remoção.
    let mic = scriba::media::extract_audio(
        &fixture("voce-com-eco.wav"),
        |_| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    let mic_alone: Vec<String> = Asr::new(vec![Plan::cpu()], base_models())
        .transcribe(&mic, |_| {}, &AtomicBool::new(false))
        .unwrap()
        .segments
        .iter()
        .map(|s| transcript::merge::words(&s.text).join(" "))
        .collect();
    println!("microfone sozinho: {mic_alone:?}");
    assert!(
        hits(&mic_alone, &OUTROS_WORDS) >= 2,
        "o eco não foi transcrito: {mic_alone:?}"
    );
    let parsed = transcript::parse(&txt);
    assert!(parsed.recording, "{txt}");
    assert_eq!(parsed.duration.as_deref(), Some("00:00:29"));
    assert!(parsed.blocks.iter().all(|b| b.speaker.is_some()), "{txt}");
    let voce = speaker_blocks(&txt, transcript::Speaker::Voce);
    let outros = speaker_blocks(&txt, transcript::Speaker::Outros);
    assert!(hits(&outros, &OUTROS_WORDS) >= 3, "{txt}");
    assert!(hits(&voce, &VOCE_WORDS) >= 3, "{txt}");
    assert_eq!(hits(&voce, &OUTROS_WORDS), 0, "eco em \"Você\": {txt}");
    // Cada fala de "Outros" uma vez só, e cada uma de "Você" presente.
    let all_voce = voce.join(" ");
    for phrase in OUTROS {
        let s = transcript::merge::similarity(phrase, &all_voce);
        assert!(s < 0.5, "{phrase:?} repetida como Você ({s:.2}): {txt}");
    }
    for phrase in VOCE {
        let s = transcript::merge::similarity(phrase, &all_voce);
        assert!(s >= 0.6, "{phrase:?} faltou ({s:.2}): {txt}");
    }

    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(outputs.dir.join(transcript::JOB_FILE)).unwrap())
            .unwrap();
    assert_eq!(manifest.media, MediaKind::Recording);
    assert_eq!(manifest.name, "Gravação de 20/10, 09:15");
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!left.iter().any(|n| n.ends_with(".f32")), "{left:?}");

    let chosen = outputs.dir.join("2026-10-20 0915 - Gravação.m4a");
    let answer = chosen.clone();
    setup
        .jobs
        .save_audio(id, move |_, _| Ok(Some(answer)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !matches!(
        setup.jobs.get(id).unwrap().state,
        State::Done {
            audio: Audio::Saved { .. },
            ..
        }
    ) {
        assert!(Instant::now() < deadline, "o áudio não terminou");
        thread::sleep(Duration::from_millis(20));
    }
    let probe = scriba::media::probe(&chosen).unwrap();
    assert!(
        (probe.duration_ms as i64 - 29_479).abs() <= 100,
        "{probe:?}"
    );
    assert!(!dir.exists(), "guardado apaga a gravação");
}

/// As trilhas reais da etapa 6.1 (`%TEMP%\scriba-gravacao\silencio`: 15 s da Câmara nos
/// alto-falantes, 15 s sem nada, 15 s de novo, depois os marcadores) com a "Você" da Maria
/// (`voce.wav`) misturada no microfone real a partir de 16 s: a 1ª fala dela cai no trecho
/// sem nada tocando, a 2ª por cima da Câmara. Roda no backend do app (a GPU dedicada).
///
/// `cargo test --release --features vulkan --test engine -- --ignored --nocapture gravacao_real`
/// (`SCRIBA_REC_DIR` troca a pasta da gravação).
#[test]
#[ignore = "hardware"]
fn gravacao_real() {
    use scriba::capture::{End, Track, storage};
    let source = std::env::var("SCRIBA_REC_DIR").map_or_else(
        |_| {
            let root = std::env::temp_dir()
                .join("scriba-gravacao")
                .join("silencio");
            std::fs::read_dir(&root)
                .unwrap_or_else(|e| panic!("{}: {e} (rode a captura da 6.1)", root.display()))
                .map(|e| e.unwrap().path())
                .find(|p| p.join(storage::MANIFEST_FILE).is_file())
                .expect("nenhuma gravação")
        },
        PathBuf::from,
    );
    let tmp = tempfile::tempdir().unwrap();
    let id = JobId::generate();
    let dir = storage::create(tmp.path(), id).unwrap();
    for track in Track::ALL {
        std::fs::copy(source.join(track.file()), dir.join(track.file())).unwrap();
    }
    let mut manifest = storage::read_manifest(&source).unwrap();
    manifest.id = id.to_string();
    storage::write_manifest(&dir, &manifest).unwrap();

    // A Maria no microfone real, a partir de 16 s.
    const AT: usize = 16 * 32_000;
    let mic_path = dir.join(Track::Mic.file());
    let mut mic: Vec<i16> = hound::WavReader::open(&mic_path)
        .unwrap()
        .samples::<i16>()
        .map(Result::unwrap)
        .collect();
    let voce: Vec<i16> = hound::WavReader::open(fixture("voce.wav"))
        .unwrap()
        .samples::<i16>()
        .map(Result::unwrap)
        .collect();
    for (m, v) in mic[AT..].iter_mut().zip(&voce) {
        *m = m.saturating_add(*v);
    }
    let mut w = hound::WavWriter::create(&mic_path, storage::wav_spec()).unwrap();
    for s in &mic {
        w.write_sample(*s).unwrap();
    }
    w.finalize().unwrap();
    let rec = storage::load(&dir, End::Recovered).unwrap();
    println!(
        "{}: {:.1} s, microfone {:?}, saída {:?}",
        source.display(),
        rec.duration_ms as f64 / 1_000.0,
        rec.mic,
        rec.output
    );

    let out = std::env::temp_dir()
        .join("scriba-gravacao")
        .join("transcricoes");
    let engine = Engine::new(out);
    println!("{}", engine.backend().unwrap_or_default());
    let job = Job::recording(rec, None);
    let ctx = Ctx::new(Arc::new(AtomicBool::new(false)), |_, _| {});
    let t0 = Instant::now();
    let outputs = engine.run(&job, &ctx).unwrap();
    let took = t0.elapsed();
    let txt = std::fs::read_to_string(&outputs.txt).unwrap();
    println!("{txt}\n{} em {took:.1?}", outputs.txt.display());

    // A trilha do sistema sozinha, no mesmo motor: "Outros" sai igual (a mescla nunca a
    // muda).
    let system = scriba::media::extract_audio(
        &dir.join(Track::System.file()),
        |_| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    let asr = Asr::detect();
    let alone: Vec<String> = asr
        .transcribe(&system, |_| {}, &AtomicBool::new(false))
        .unwrap()
        .segments
        .into_iter()
        .map(|s| s.text)
        .collect();
    let alone = alone.join(" ");
    let outros = speaker_blocks(&txt, transcript::Speaker::Outros).join(" ");
    let voce = speaker_blocks(&txt, transcript::Speaker::Voce);
    let same = transcript::merge::similarity(&alone, &outros);
    println!(
        "\"Outros\" × sistema.wav sozinho: {same:.2} ({} palavras)",
        outros.split(' ').count()
    );
    assert!(
        outros.split(' ').count() >= 40,
        "pouco texto em Outros: {txt}"
    );
    assert!(same >= 0.6, "{same:.2}");
    let all_voce = voce.join(" ");
    for phrase in VOCE {
        let s = transcript::merge::similarity(phrase, &all_voce);
        println!("\"Você\" × {phrase:?}: {s:.2}");
        assert!(s >= 0.6, "{phrase:?} faltou ({s:.2})");
    }
    for block in &voce {
        let s = transcript::merge::similarity(block, &outros);
        println!("\"Você\" {block:?} × Outros: {s:.2}");
        assert!(s < 0.6, "parágrafo \"Você\" repete \"Outros\": {block:?}");
    }
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

/// Meta da etapa 4.6: o pico de RAM em qualquer duração.
const MAX_RAM: u64 = 1_536 << 20;

/// Transcreve o áudio longo de `var` em cada backend pedido, com o `Engine` real, e imprime
/// tempo, RTF e pico de RAM. Afirma as metas de 1 h de áudio (≤ 10 min na GPU dedicada,
/// ≤ 1 h na CPU, proporcionais à duração) e o pico de RAM.
fn long_audio(var: &str) {
    let path = PathBuf::from(
        std::env::var(var).unwrap_or_else(|_| panic!("defina {var} com um áudio longo")),
    );
    let devices = std::env::var("SCRIBA_LONG_DEVICES").unwrap_or_else(|_| "gpu,cpu".into());
    let block = std::env::var("SCRIBA_LONG_BLOCK_MIN")
        .ok()
        .map(|m| Duration::from_secs(m.parse::<u64>().expect("minutos") * 60));
    let out = std::env::temp_dir().join("scriba-longo");
    let dir = tempfile::tempdir().unwrap();
    // O `Engine` grava o áudio extraído ao lado do envio: um link no temporário evita
    // copiar o arquivo e não suja a pasta dele.
    let input = dir.path().join(format!(
        "entrada.{}",
        path.extension().unwrap().to_str().unwrap()
    ));
    if std::fs::hard_link(&path, &input).is_err() {
        std::fs::copy(&path, &input).unwrap();
    }
    let name = path.file_name().unwrap().to_str().unwrap().to_string();

    let mut rows = Vec::new();
    for device in devices.split(',').map(str::trim) {
        let models = Models::locate();
        let plans: Vec<Plan> = asr::plans(&asr::devices(), &models)
            .into_iter()
            .filter(|p| match device {
                "gpu" => matches!(p.backend, Backend::Gpu(_)),
                "cpu" => p.backend == Backend::Cpu,
                other => panic!("dispositivo desconhecido: {other}"),
            })
            .collect();
        assert!(!plans.is_empty(), "sem candidato para {device}");
        let mut asr = Asr::new(plans, models);
        if let Some(block) = block {
            asr = asr.blocks(block);
        }
        let engine = Engine::with_asr(out.clone(), asr);
        let probe = engine.probe(&name, &input).unwrap();
        let job = Job {
            id: JobId::generate(),
            name: name.clone(),
            media: probe.media,
            size: std::fs::metadata(&input).unwrap().len(),
            duration_ms: probe.duration_ms,
            created: created(),
            estimate: probe.estimate,
            input: input.clone(),
            state: State::Queued,
        };
        let last = Mutex::new((Stage::ExtractingAudio, 0u8));
        let t0 = Instant::now();
        let label = device.to_string();
        let ctx = Ctx::new(Arc::new(AtomicBool::new(false)), move |stage, pct| {
            let mut last = last.lock().unwrap();
            if stage != last.0 || pct >= last.1 + 10 || pct == 100 {
                println!("{label}: {stage:?} {pct}% em {:.0?}", t0.elapsed());
                *last = (stage, pct);
            }
        });
        let baseline = working_set();
        let sampler = Sampler::start();
        let outputs = engine.run(&job, &ctx).unwrap();
        let peak = sampler.stop();
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(outputs.dir.join(transcript::JOB_FILE)).unwrap())
                .unwrap();
        assert!(!scriba::engine::spool_path(&input).exists());
        let t = manifest.timings;
        let audio_secs = manifest.duration_ms as f64 / 1_000.0;
        let row = format!(
            "| {name} | {device} ({}) | {:.0} min | {:.1} s | {:.1} s | {:.1} s | {:.3} | {} MB | {} MB | {} |",
            manifest.model,
            audio_secs / 60.0,
            t.extract_ms as f64 / 1_000.0,
            t.transcribe_ms as f64 / 1_000.0,
            t.total_ms as f64 / 1_000.0,
            t.total_ms as f64 / 1_000.0 / audio_secs,
            baseline >> 20,
            peak >> 20,
            outputs.txt.display(),
        );
        println!("{row}");
        rows.push(row.clone());
        let hours = audio_secs / 3_600.0;
        let limit = if device == "gpu" { 600.0 } else { 3_600.0 } * hours;
        assert!(t.total_ms as f64 / 1_000.0 <= limit, "{row}");
        assert!(peak <= MAX_RAM, "pico de {} MB: {row}", peak >> 20);
        drop(engine);
    }
    println!(
        "\n| Arquivo | Backend | Duração | Extração | Transcrição | Total | RTF | RAM antes | Pico de RAM | Saída |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for row in rows {
        println!("{row}");
    }
}

/// 1 h de áudio real: tempo e pico de RAM na GPU dedicada e na CPU.
#[test]
#[ignore = "hardware"]
fn uma_hora() {
    long_audio("SCRIBA_LONG_AUDIO");
}

/// 3 h de áudio real: o pico de RAM continua o de um bloco.
#[test]
#[ignore = "hardware"]
fn tres_horas() {
    long_audio("SCRIBA_LONG_AUDIO_3H");
}
