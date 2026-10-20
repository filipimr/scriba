//! `asr` com o whisper de verdade: o modelo base na CPU, como no CI
//! (`cargo xtask model --only base,silero`), com as fixtures. Os de hardware usam a RTX e a
//! Intel: `cargo test --release --features vulkan --test asr -- --ignored`.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{audio, base_models, keyword_hits};
use scriba::asr::{self, Asr, Backend, Device, DeviceKind, Error, ModelKind, Models, Plan, Source};
use scriba::transcript::{self, Segment};

fn base_cpu() -> Asr {
    Asr::new(vec![Plan::cpu()], base_models())
}

fn text(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// Segmentos em ordem, dentro do áudio e com fim depois do início.
fn assert_timeline(segments: &[Segment], samples: usize) {
    let total_ms = (samples / 16) as u64;
    let mut previous = 0;
    for s in segments {
        assert!(s.start_ms >= previous, "{segments:?}");
        assert!(
            s.start_ms <= s.end_ms && s.end_ms <= total_ms,
            "{segments:?}"
        );
        previous = s.start_ms;
    }
}

#[test]
fn formatos_dao_as_palavras_chave() {
    let asr = base_cpu();
    for name in ["fala.mp4", "fala.mp3", "fala.m4a", "fala.ogg", "fala.wav"] {
        let samples = audio(name);
        let t0 = Instant::now();
        let result = asr.transcribe(&samples, |_| {}, &never()).unwrap();
        let text = text(&result.segments);
        println!("{name}: {:.2?} → {text}", t0.elapsed());
        assert!(keyword_hits(&text) >= 4, "{name}: {text}");
        assert_timeline(&result.segments, samples.len());
        assert_eq!((result.plan.clone(), result.model), (Plan::cpu(), "base"));
    }
}

#[test]
fn silencio_da_texto_vazio() {
    let mut seen = Vec::new();
    let result = base_cpu()
        .transcribe(&audio("silencio.wav"), |p| seen.push(p), &never())
        .unwrap();
    assert_eq!(result.segments, []);
    assert_eq!(seen, [0, 100]);
}

#[test]
fn progresso_cresce_e_chega_a_100() {
    let mut seen = Vec::new();
    // 6 × 10,9 s: mais de uma janela de 30 s do whisper.
    let samples = audio("fala.wav").repeat(6);
    base_cpu()
        .transcribe(&samples, |p| seen.push(p), &never())
        .unwrap();
    assert_eq!(seen.first(), Some(&0), "{seen:?}");
    assert_eq!(seen.last(), Some(&100), "{seen:?}");
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
    assert!(seen.len() > 2, "{seen:?}");
}

#[test]
fn cancelar_interrompe_o_whisper() {
    let samples = audio("fala.wav").repeat(6);
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let t0 = Instant::now();
    // Cancela no primeiro progresso do whisper, depois do VAD: o abort interrompe.
    let result = base_cpu().transcribe(
        &samples,
        |p| {
            seen.push(p);
            if p > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    );
    println!("cancelado em {:.2?}: {seen:?}", t0.elapsed());
    assert_eq!(result.unwrap_err(), Error::Canceled);
    assert!(seen.last().is_some_and(|&p| p > 0 && p < 100), "{seen:?}");
}

#[test]
fn cancelado_antes_nem_carrega() {
    let asr = base_cpu();
    let err = asr
        .transcribe(&audio("fala.wav"), |_| {}, &AtomicBool::new(true))
        .unwrap_err();
    assert_eq!(err, Error::Canceled);
    assert!(!asr.is_loaded());
}

#[test]
fn pausa_longa_quebra_o_paragrafo() {
    let samples = audio("pausas.wav");
    let result = base_cpu().transcribe(&samples, |_| {}, &never()).unwrap();
    assert_timeline(&result.segments, samples.len());
    // Sem o mapa do VAD, os 3 s de silêncio virariam 0,1 s e o parágrafo não quebraria.
    let paragraphs = transcript::paragraphs(&result.segments);
    println!("{paragraphs:?}");
    assert_eq!(paragraphs.len(), 2, "{:?}", result.segments);
    assert!(paragraphs[0].text.to_lowercase().contains("orçamento"));
    assert!(paragraphs[1].text.to_lowercase().contains("cronograma"));
    // A segunda frase começa depois dos 3 s de pausa (a primeira dura ~3 s).
    assert!(paragraphs[1].start_ms >= 5_000, "{paragraphs:?}");
}

/// Bloco reduzido, para o áudio de ~37 s abaixo passar por três blocos.
const BLOCO: Duration = Duration::from_secs(12);

/// `fala.wav` + 1 s de silêncio + `pausas.wav` + `fala.wav` (~37 s) e onde começa cada
/// `fala.wav`, em ms.
fn longo() -> (Vec<f32>, [u64; 2]) {
    let fala = audio("fala.wav");
    let mut samples = fala.clone();
    samples.extend(std::iter::repeat_n(0.0, 16_000));
    samples.extend(audio("pausas.wav"));
    let last = (samples.len() / 16) as u64;
    samples.extend(&fala);
    (samples, [0, last])
}

/// O texto dos segmentos que começam em `range` (ms).
fn text_in(segments: &[Segment], range: std::ops::Range<u64>) -> String {
    let inside: Vec<_> = segments
        .iter()
        .filter(|s| range.contains(&s.start_ms))
        .cloned()
        .collect();
    text(&inside)
}

#[test]
fn blocos_mantem_os_tempos_e_os_paragrafos() {
    let (samples, [_, last]) = longo();
    let mut seen = Vec::new();
    let blocks = base_cpu()
        .blocks(BLOCO)
        .transcribe(&samples, |p| seen.push(p), &never())
        .unwrap();
    let whole = base_cpu().transcribe(&samples, |_| {}, &never()).unwrap();
    for (name, segments) in [("blocos", &blocks.segments), ("inteiro", &whole.segments)] {
        println!("{name}:");
        for s in segments {
            println!("  {:>6} {:>6} {}", s.start_ms, s.end_ms, s.text);
        }
    }

    // Tempos do áudio inteiro, não do bloco: crescentes e dentro da duração.
    assert_timeline(&blocks.segments, samples.len());
    let head = text_in(&blocks.segments, 0..11_000);
    let tail = text_in(&blocks.segments, last..u64::MAX);
    assert!(keyword_hits(&head) >= 4, "{head}");
    assert!(keyword_hits(&tail) >= 4, "{tail}");
    assert!(
        text_in(&blocks.segments, 11_000..last)
            .to_lowercase()
            .contains("cronograma"),
        "{:?}",
        blocks.segments
    );

    // A emenda não quebra nem junta parágrafos: as pausas são as do original.
    let paragraphs = |segments| {
        transcript::paragraphs(segments)
            .iter()
            .map(|p| p.start_ms / 1_000)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        paragraphs(&blocks.segments).len(),
        paragraphs(&whole.segments).len(),
        "{:?} × {:?}",
        paragraphs(&blocks.segments),
        paragraphs(&whole.segments)
    );

    // O progresso atravessa os blocos sem voltar.
    assert_eq!(seen.first(), Some(&0), "{seen:?}");
    assert_eq!(seen.last(), Some(&100), "{seen:?}");
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
    assert!(seen.len() >= 4, "{seen:?}");
}

#[test]
fn cancelar_no_segundo_bloco() {
    let (samples, _) = longo();
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    // O 1º bloco termina no silêncio depois do 1º `fala.wav` (~11 s de ~37 s): o primeiro
    // progresso acima de 0 já é do 2º bloco.
    let result = base_cpu().blocks(BLOCO).transcribe(
        &samples,
        |p| {
            seen.push(p);
            if p > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    );
    println!("{seen:?}");
    assert_eq!(result.unwrap_err(), Error::Canceled);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!((20..50).contains(&seen[1]), "{seen:?}");
}

#[test]
fn gpu_que_nao_existe_cai_para_a_cpu() {
    // O whisper.cpp não acha o dispositivo 99, loga "no GPU found" e seguiria na CPU sem
    // erro; a conferência pelo log descarta o candidato e o próximo assume.
    let ghost = Plan {
        backend: Backend::Gpu(Device {
            index: 99,
            name: "Vulkan99".into(),
            description: "GPU inexistente".into(),
            kind: DeviceKind::Dedicated,
            memory: 8 << 30,
        }),
        model: ModelKind::Cpu,
        flash_attn: true,
    };
    let asr = Asr::new(vec![ghost, Plan::cpu()], base_models());
    assert_eq!(asr.preload(), Ok(Plan::cpu()));
    let result = asr
        .transcribe(&audio("fala.wav"), |_| {}, &never())
        .unwrap();
    assert_eq!(result.plan, Plan::cpu());
    assert!(keyword_hits(&text(&result.segments)) >= 4);
}

#[test]
fn sem_vulkan_o_plano_e_a_cpu() {
    let devices = asr::devices();
    let registry = asr::registry();
    println!("backends: {registry:?}; GPUs: {devices:?}");
    assert!(registry.iter().any(|r| r == "CPU"), "{registry:?}");
    if !registry.iter().any(|r| r == "Vulkan") {
        assert_eq!(devices, []);
    }
    let plans = asr::plans(&devices, &base_models());
    // Só com o modelo da CPU, as GPUs dedicadas (modelo grande) ficam de fora.
    assert_eq!(plans.last(), Some(&Plan::cpu()));
    assert!(
        plans
            .iter()
            .all(|p| !matches!(&p.backend, Backend::Gpu(d) if d.kind == DeviceKind::Dedicated))
    );
}

#[test]
fn modelo_ausente_da_erro_amigavel() {
    let missing = Source::File(common::models_dir().join("nao-existe.bin"));
    let mut no_model = base_models();
    no_model.cpu.as_mut().unwrap().source = missing.clone();
    let mut no_vad = base_models();
    no_vad.vad = Some(missing);
    for models in [no_model, no_vad, Models::default()] {
        let plans = asr::plans(&[], &models);
        let asr = Asr::new(plans, models);
        let err = asr
            .transcribe(&audio("fala.wav"), |_| {}, &never())
            .unwrap_err();
        assert_eq!(err, Error::Load);
        assert!(!asr.is_loaded());
    }
}

#[test]
fn libera_o_modelo_depois_de_ocioso() {
    let asr = Asr::with_idle(vec![Plan::cpu()], base_models(), Duration::from_millis(100));
    assert!(!asr.is_loaded());
    asr.preload().unwrap();
    assert!(asr.is_loaded());
    let deadline = Instant::now() + Duration::from_secs(10);
    while asr.is_loaded() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!asr.is_loaded());
    // Recarrega sozinho na próxima transcrição.
    let result = asr
        .transcribe(&audio("fala.wav"), |_| {}, &never())
        .unwrap();
    assert!(keyword_hits(&text(&result.segments)) >= 4);
}

#[test]
fn panico_no_progresso_nao_derruba_o_processo() {
    let samples = audio("fala.wav").repeat(6);
    let asr = base_cpu();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        asr.transcribe(
            &samples,
            |p| {
                sink.lock().unwrap().push(p);
                assert!(p == 0, "pânico simulado no progresso");
            },
            &never(),
        )
    }));
    assert!(outcome.is_err(), "o pânico continua depois do whisper");
    // O whisper parou no primeiro progresso depois do pânico, e o motor segue usável.
    assert!(!seen.lock().unwrap().contains(&100));
    let result = asr
        .transcribe(&audio("fala.wav"), |_| {}, &never())
        .unwrap();
    assert!(keyword_hits(&text(&result.segments)) >= 4);
}

// ---- Suíte de hardware: RTX 3050 (dedicada) e Intel (integrada) desta máquina ----

/// O modelo grande em cada GPU, a integrada inclusive (fora do `plans()`, mas o teste mede o
/// dispositivo), com flash attention só na dedicada; a dedicada primeiro.
fn gpu_plans() -> Vec<Plan> {
    let mut devices = asr::devices();
    devices.sort_by_key(|d| d.kind != DeviceKind::Dedicated);
    devices
        .into_iter()
        .map(|d| Plan {
            flash_attn: d.kind == DeviceKind::Dedicated,
            backend: Backend::Gpu(d),
            model: ModelKind::Large,
        })
        .collect()
}

#[test]
#[ignore = "hardware"]
fn enumera_a_intel_como_integrada_e_a_rtx_como_dedicada() {
    let devices = asr::devices();
    println!("{devices:#?}");
    assert!(devices.iter().any(|d| d.kind == DeviceKind::Integrated));
    assert!(devices.iter().any(|d| d.kind == DeviceKind::Dedicated));
    let plans = asr::plans(&devices, &Models::locate());
    let Backend::Gpu(first) = &plans[0].backend else {
        panic!("{plans:?}");
    };
    assert_eq!(first.kind, DeviceKind::Dedicated);
    assert!(plans[0].flash_attn && plans[0].model == ModelKind::Large);
    assert_eq!(plans.last(), Some(&Plan::cpu()));
}

#[test]
#[ignore = "hardware"]
fn large_na_rtx_e_na_intel() {
    let samples = audio("fala.wav");
    for plan in gpu_plans() {
        let asr = Asr::new(vec![plan.clone()], Models::locate());
        let t0 = Instant::now();
        assert_eq!(asr.preload(), Ok(plan.clone()));
        let load = t0.elapsed();
        let t1 = Instant::now();
        let result = asr.transcribe(&samples, |_| {}, &never()).unwrap();
        let text = text(&result.segments);
        println!(
            "{plan}: carga {load:.2?}, transcrição {:.2?} → {text}",
            t1.elapsed()
        );
        assert_eq!(result.plan, plan);
        assert!(keyword_hits(&text) >= 4, "{plan}: {text}");
    }
}

#[test]
#[ignore = "hardware"]
fn libera_a_memoria_da_gpu_depois_de_ocioso() {
    let plan = gpu_plans().into_iter().next().expect("sem GPU");
    let Backend::Gpu(device) = plan.backend.clone() else {
        unreachable!()
    };
    let free = || asr::free_memory(&device).unwrap() as f64 / f64::from(1 << 20);
    let before = free();
    let asr = Asr::with_idle(vec![plan], Models::locate(), Duration::from_secs(2));
    asr.preload().unwrap();
    let loaded = free();
    let deadline = Instant::now() + Duration::from_secs(30);
    while asr.is_loaded() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    // O driver devolve a memória logo depois da liberação.
    std::thread::sleep(Duration::from_millis(500));
    let after = free();
    println!(
        "{}: livre {before:.0} MB → carregado {loaded:.0} MB → liberado {after:.0} MB",
        device.description
    );
    assert!(!asr.is_loaded());
    assert!(before - loaded > 300.0, "o modelo grande ocupa a GPU");
    assert!(after - loaded > 0.8 * (before - loaded), "a memória voltou");
}
