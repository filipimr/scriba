//! Benchmark de transcrição (suíte de hardware): modelos × dispositivos × flash attention ×
//! threads, com uma tabela Markdown no fim. Meça com o build de release, na tomada:
//!
//! `cargo test -p spikes --release --features vulkan --test bench -- --ignored --nocapture`
//!
//! Parâmetros, em listas separadas por vírgula:
//! - `SCRIBA_BENCH_MODELS`: `base`, `small` e `large` (padrão: os três).
//! - `SCRIBA_BENCH_DEVICES`: `cpu` e índices Vulkan (padrão: `cpu`).
//! - `SCRIBA_BENCH_FLASH`: `0` e/ou `1` (padrão: `0`).
//! - `SCRIBA_BENCH_THREADS`: threads da CPU (padrão: núcleos lógicos − 2).
//! - `SCRIBA_BENCH_AUDIO`: WAV 16 kHz mono (padrão: `tests/fixtures/fala.wav`).
//! - `SCRIBA_BENCH_REPEAT`: repete o áudio N vezes (padrão: 1), para medir mais de uma
//!   janela de 30 s.

use std::path::PathBuf;
use std::time::Instant;

use spikes::{default_threads, fixture, keyword_hits, model, read_wav_f32, transcribe_with};
use whisper_rs::{WhisperContext, WhisperContextParameters};

fn list(name: &str, default: &str) -> Vec<String> {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn model_file(name: &str) -> &'static str {
    match name {
        "base" => "ggml-base-q5_1.bin",
        "small" => "ggml-small-q5_1.bin",
        "large" => "ggml-large-v3-turbo-q5_0.bin",
        other => panic!("modelo desconhecido: {other} (use base, small ou large)"),
    }
}

#[test]
#[ignore = "hardware"]
fn benchmark() {
    // Sem backend de log no whisper-rs, os hooks só silenciam o whisper.cpp e o ggml.
    whisper_rs::install_logging_hooks();
    println!("whisper.cpp {}", whisper_rs::WHISPER_CPP_VERSION);
    println!("{}", whisper_rs::print_system_info());
    #[cfg(feature = "vulkan")]
    for d in whisper_rs::vulkan::list_devices() {
        println!("Vulkan {}: {} ({} MB)", d.id, d.name, d.vram.total >> 20);
    }

    let custom_audio = std::env::var("SCRIBA_BENCH_AUDIO").ok().map(PathBuf::from);
    let audio_path = custom_audio.clone().unwrap_or_else(|| fixture("fala.wav"));
    let repeat: usize = std::env::var("SCRIBA_BENCH_REPEAT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let audio = read_wav_f32(&audio_path).unwrap().repeat(repeat.max(1));
    let secs = audio.len() as f64 / 16_000.0;
    println!("áudio: {} × {repeat} = {secs:.1} s", audio_path.display());

    let threads: Vec<i32> = list("SCRIBA_BENCH_THREADS", &default_threads().to_string())
        .iter()
        .map(|t| t.parse().expect("SCRIBA_BENCH_THREADS"))
        .collect();
    let mut rows = Vec::new();
    for name in list("SCRIBA_BENCH_MODELS", "base,small,large") {
        let bytes = std::fs::read(model(model_file(&name)))
            .unwrap_or_else(|e| panic!("{name}: {e} (rode `cargo xtask model`)"));
        for device in list("SCRIBA_BENCH_DEVICES", "cpu") {
            for flash in list("SCRIBA_BENCH_FLASH", "0") {
                let gpu = (device != "cpu").then(|| device.parse::<i32>().expect("dispositivo"));
                let mut p = WhisperContextParameters::default();
                p.use_gpu(gpu.is_some());
                p.gpu_device(gpu.unwrap_or(0));
                p.flash_attn(flash == "1");
                let t0 = Instant::now();
                let ctx = WhisperContext::new_from_buffer_with_params(&bytes, p).unwrap();
                let load = t0.elapsed().as_secs_f64();
                for &n in &threads {
                    let t1 = Instant::now();
                    let text = transcribe_with(&ctx, &audio, n).unwrap();
                    let run = t1.elapsed().as_secs_f64();
                    let hits = if custom_audio.is_none() {
                        format!("{}/6", keyword_hits(&text))
                    } else {
                        "—".into()
                    };
                    let label = gpu.map_or("CPU".into(), |i| format!("Vulkan {i}"));
                    println!("{name} {label} flash={flash} t={n}: {run:.2} s → {text}");
                    rows.push(format!(
                        "| {name} | {label} | {flash} | {n} | {load:.2} s | {run:.2} s | {:.3} | {:.1}× | {hits} |",
                        run / secs,
                        secs / run
                    ));
                }
            }
        }
    }
    println!();
    println!(
        "| Modelo | Dispositivo | Flash | Threads | Carga | Transcrição | RTF | Tempo real | Palavras-chave |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for row in rows {
        println!("{row}");
    }
}
