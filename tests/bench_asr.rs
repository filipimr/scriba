//! Benchmark small × base (e large) com as configurações do app, na suíte de hardware: WER,
//! fator de tempo real (RTF), carga e pico de RAM por modelo e dispositivo. Meça com o build
//! de release, na tomada e em "Melhor desempenho":
//!
//! `cargo test --release --features vulkan --test bench_asr -- --ignored --nocapture`
//!
//! Parâmetros, em listas separadas por vírgula:
//! - `SCRIBA_BENCH_MODELS`: `base`, `small` e `large` (padrão: os três).
//! - `SCRIBA_BENCH_DEVICES`: `cpu`, `igpu` (a integrada) e `gpu` (a dedicada); padrão: os
//!   que existirem.
//! - `SCRIBA_BENCH_AUDIO`: áudios ou vídeos. A referência de cada um é o texto em
//!   `<nome>.ref.txt` ao lado dele ou, sem esse arquivo, a transcrição do large na GPU
//!   dedicada. Sem a variável, as fixtures `fala.wav` e `pausas.wav`, de texto conhecido.
//! - `SCRIBA_BENCH_BEAM`: um tamanho de beam (ex.: `5`) para medir também o beam search,
//!   só como referência.
//!
//! As transcrições ficam em `%TEMP%\scriba-bench-asr`, para comparar os textos.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::{Duration, Instant};

use common::{Sampler, memory, working_set};
use scriba::asr::{self, Asr, Backend, DeviceKind, Model, ModelKind, Models, Plan, Source};

fn list(name: &str, default: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

struct Audio {
    name: String,
    samples: Vec<f32>,
    /// Texto certo; `None` usa o large na GPU dedicada.
    reference: Option<String>,
}

impl Audio {
    fn secs(&self) -> f64 {
        self.samples.len() as f64 / 16_000.0
    }
}

fn audios() -> Vec<Audio> {
    let Ok(paths) = std::env::var("SCRIBA_BENCH_AUDIO") else {
        return vec![
            Audio {
                name: "fala.wav".into(),
                samples: common::audio("fala.wav"),
                reference: Some(common::FALA.into()),
            },
            Audio {
                name: "pausas.wav".into(),
                samples: common::audio("pausas.wav"),
                reference: Some(common::PAUSAS.join(" ")),
            },
        ];
    };
    paths
        .split(',')
        .map(|p| PathBuf::from(p.trim()))
        .map(|path| {
            let samples = scriba::media::extract_audio(&path, |_| {}, &AtomicBool::new(false))
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let reference = std::fs::read_to_string(path.with_extension("ref.txt")).ok();
            Audio {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                samples,
                reference,
            }
        })
        .collect()
}

/// Um modelo num dispositivo, com os parâmetros do app.
#[derive(Clone)]
struct Config {
    model: String,
    plan: Plan,
    models: Models,
    beam: Option<i32>,
}

impl Config {
    fn new(model: &str, device: &str, beam: Option<i32>) -> Option<Self> {
        let dir = common::models_dir();
        let (kind, name, file) = match model {
            "large" => (ModelKind::Large, "large-v3-turbo", asr::LARGE_FILE),
            "small" => (ModelKind::Cpu, "small", asr::SMALL_FILE),
            "base" => (ModelKind::Cpu, "base", asr::BASE_FILE),
            other => panic!("modelo desconhecido: {other} (use base, small ou large)"),
        };
        let backend = match device {
            "cpu" => Backend::Cpu,
            "igpu" | "gpu" => {
                let kind = if device == "gpu" {
                    DeviceKind::Dedicated
                } else {
                    DeviceKind::Integrated
                };
                Backend::Gpu(asr::devices().into_iter().find(|d| d.kind == kind)?)
            }
            other => panic!("dispositivo desconhecido: {other} (use cpu, igpu ou gpu)"),
        };
        let model_entry = Model {
            name,
            source: Source::File(dir.join(file)),
        };
        assert!(
            dir.join(file).is_file(),
            "{file} ausente: rode `cargo xtask model`"
        );
        Some(Self {
            model: model.into(),
            plan: Plan {
                // Flash attention só na dedicada, como no app (etapa 4.1).
                flash_attn: matches!(&backend, Backend::Gpu(d) if d.kind == DeviceKind::Dedicated),
                backend,
                model: kind,
            },
            models: Models {
                large: (kind == ModelKind::Large).then(|| model_entry.clone()),
                cpu: (kind == ModelKind::Cpu).then_some(model_entry),
                vad: Some(Source::File(dir.join(asr::VAD_FILE))),
            },
            beam,
        })
    }

    fn label(&self) -> String {
        match self.beam {
            Some(b) => format!("{} (beam {b})", self.model),
            None => self.model.clone(),
        }
    }
}

struct Run {
    load: f64,
    peak_mb: f64,
    /// Por áudio: segundos e texto.
    results: Vec<(f64, String)>,
}

fn run(config: &Config, audios: &[Audio]) -> Run {
    let baseline = working_set();
    let sampler = Sampler::start();
    let mut asr = Asr::new(vec![config.plan.clone()], config.models.clone());
    if let Some(beam) = config.beam {
        asr = asr.beam(beam);
    }
    let t0 = Instant::now();
    assert_eq!(asr.preload(), Ok(config.plan.clone()), "{}", config.plan);
    let load = t0.elapsed().as_secs_f64();
    // O primeiro uso do Vulkan no processo compila os shaders: fora da medida.
    let warmup = &common::audio("fala.wav");
    asr.transcribe(warmup, |_| {}, &AtomicBool::new(false))
        .unwrap();
    let results = audios
        .iter()
        .map(|audio| {
            let t = Instant::now();
            let result = asr
                .transcribe(&audio.samples, |_| {}, &AtomicBool::new(false))
                .unwrap();
            let secs = t.elapsed().as_secs_f64();
            let text: Vec<&str> = result.segments.iter().map(|s| s.text.as_str()).collect();
            (secs, text.join("\n"))
        })
        .collect();
    drop(asr);
    let peak_mb = sampler.stop().saturating_sub(baseline) as f64 / f64::from(1 << 20);
    Run {
        load,
        peak_mb,
        results,
    }
}

#[test]
#[ignore = "hardware"]
fn benchmark() {
    println!("whisper.cpp {}", whisper_rs::WHISPER_CPP_VERSION);
    println!("{}", whisper_rs::print_system_info());
    for d in asr::devices() {
        println!(
            "{}: {} ({:?}, {} MB)",
            d.name,
            d.description,
            d.kind,
            d.memory >> 20
        );
    }
    let mut audios = audios();
    for a in &audios {
        println!("áudio: {} ({:.1} s)", a.name, a.secs());
    }
    let out_dir = std::env::temp_dir().join("scriba-bench-asr");
    std::fs::create_dir_all(&out_dir).unwrap();

    // A referência dos áudios sem texto: o large na GPU dedicada.
    if audios.iter().any(|a| a.reference.is_none()) {
        let reference =
            Config::new("large", "gpu", None).expect("sem GPU dedicada para a referência");
        let run = run(&reference, &audios);
        for (audio, (_, text)) in audios.iter_mut().zip(run.results) {
            if audio.reference.is_none() {
                save(&out_dir, &audio.name, "referencia", &text);
                audio.reference = Some(text);
            }
        }
    }

    let devices = list("SCRIBA_BENCH_DEVICES", "cpu,igpu,gpu");
    let beams: Vec<Option<i32>> = std::iter::once(None)
        .chain(
            std::env::var("SCRIBA_BENCH_BEAM")
                .ok()
                .map(|b| Some(b.parse().expect("SCRIBA_BENCH_BEAM"))),
        )
        .collect();
    let mut rows = Vec::new();
    let mut summary = Vec::new();
    for model in list("SCRIBA_BENCH_MODELS", "base,small,large") {
        for device in &devices {
            for &beam in &beams {
                let Some(config) = Config::new(&model, device, beam) else {
                    println!("{device}: não há esse dispositivo");
                    continue;
                };
                println!("medindo {} em {}…", config.label(), config.plan);
                let run = run(&config, &audios);
                let (mut secs, mut audio_secs, mut errors, mut words) = (0.0, 0.0, 0, 0);
                for (audio, (t, text)) in audios.iter().zip(&run.results) {
                    let reference = audio.reference.as_deref().unwrap_or_default();
                    let (e, n) = word_errors(reference, text);
                    save(
                        &out_dir,
                        &audio.name,
                        &format!("{}-{device}", config.label()),
                        text,
                    );
                    rows.push(format!(
                        "| {} | {} | {device} | {:.2} s | {t:.2} s | {:.3} | {:.1}% | {:.0} MB |",
                        audio.name,
                        config.label(),
                        run.load,
                        t / audio.secs(),
                        100.0 * e as f64 / n.max(1) as f64,
                        run.peak_mb,
                    ));
                    secs += t;
                    audio_secs += audio.secs();
                    errors += e;
                    words += n;
                }
                summary.push(format!(
                    "| {} | {device} | {:.2} s | {secs:.1} s | {:.3} | {:.1}% | {:.0} MB |",
                    config.label(),
                    run.load,
                    secs / audio_secs,
                    100.0 * errors as f64 / words.max(1) as f64,
                    run.peak_mb,
                ));
            }
        }
    }
    println!();
    println!("| Áudio | Modelo | Dispositivo | Carga | Transcrição | RTF | WER | RAM |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for row in rows {
        println!("{row}");
    }
    println!();
    println!(
        "Todos os áudios ({:.1} s):",
        audios.iter().map(Audio::secs).sum::<f64>()
    );
    println!();
    println!("| Modelo | Dispositivo | Carga | Transcrição | RTF | WER | RAM |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for row in summary {
        println!("{row}");
    }
    println!();
    println!("Transcrições em {}", out_dir.display());
}

fn save(dir: &Path, audio: &str, label: &str, text: &str) {
    let name = format!("{audio}.{}.txt", label.replace([' ', '(', ')'], ""));
    std::fs::write(dir.join(name), text).unwrap();
}

/// Palavras em minúsculas, sem pontuação; os acentos ficam.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Erros de palavra (substituições, inserções e remoções) e o total de palavras da
/// referência: WER = erros / total.
fn word_errors(reference: &str, hypothesis: &str) -> (usize, usize) {
    let (r, h) = (words(reference), words(hypothesis));
    // Distância de edição com duas linhas: um áudio de 1 h tem ~8 mil palavras.
    let mut previous: Vec<usize> = (0..=h.len()).collect();
    let mut current = vec![0; h.len() + 1];
    for (i, rw) in r.iter().enumerate() {
        current[0] = i + 1;
        for (j, hw) in h.iter().enumerate() {
            let substitution = previous[j] + usize::from(rw != hw);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    (previous[h.len()], r.len())
}

#[test]
fn wer_conta_substituicoes_insercoes_e_remocoes() {
    assert_eq!(word_errors("Bom dia, Scriba!", "bom dia scriba"), (0, 3));
    assert_eq!(word_errors("a b c d", "a x c d"), (1, 4));
    assert_eq!(word_errors("a b c d", "a b d"), (1, 4));
    assert_eq!(word_errors("a b c d", "a b c d e f"), (2, 4));
    assert_eq!(word_errors("às nove horas", "às 9 horas"), (1, 3));
    assert_eq!(word_errors("", "algo"), (1, 0));
    assert_eq!(word_errors("", ""), (0, 0));
    assert_eq!(words("Até-a  PRÓXIMA!!"), ["até", "a", "próxima"]);
}

#[test]
fn amostra_a_memoria_do_processo() {
    let (now, peak) = memory();
    assert!(now > 0 && peak >= now);
    let sampler = Sampler::start();
    let big = std::hint::black_box(vec![1u8; 64 << 20]);
    thread::sleep(Duration::from_millis(100));
    let sampled = sampler.stop();
    assert!(sampled >= now + (32 << 20), "{sampled} {now}");
    drop(big);
}
