//! Spike A: whisper.cpp transcreve pt-BR, carrega da memória e roda em CPU e Vulkan.

use std::time::Instant;

use spikes::{fixture, keyword_hits, model, read_wav_f32, transcribe};
use whisper_rs::{WhisperContext, WhisperContextParameters};

fn cpu_params() -> WhisperContextParameters<'static> {
    let mut p = WhisperContextParameters::default();
    p.use_gpu(false);
    p
}

#[test]
fn base_cpu_transcreve_fixture_pt_br() {
    let path = model("ggml-base-q5_1.bin");
    let ctx = WhisperContext::new_with_params(path.to_str().unwrap(), cpu_params()).unwrap();
    let audio = read_wav_f32(&fixture("fala.wav")).unwrap();
    let text = transcribe(&ctx, &audio).unwrap();
    assert!(keyword_hits(&text) >= 3, "texto: {text}");
}

#[test]
fn base_carrega_da_memoria() {
    let bytes = std::fs::read(model("ggml-base-q5_1.bin")).unwrap();
    let ctx = WhisperContext::new_from_buffer_with_params(&bytes, cpu_params()).unwrap();
    let audio = read_wav_f32(&fixture("fala.wav")).unwrap();
    let text = transcribe(&ctx, &audio).unwrap();
    assert!(keyword_hits(&text) >= 3, "texto: {text}");
}

#[test]
fn silencio_nao_gera_texto_longo() {
    let path = model("ggml-base-q5_1.bin");
    let ctx = WhisperContext::new_with_params(path.to_str().unwrap(), cpu_params()).unwrap();
    let audio = read_wav_f32(&fixture("silencio.wav")).unwrap();
    let text = transcribe(&ctx, &audio).unwrap();
    // Sem VAD ainda: só registra. O filtro de frases-fantasma é da Fase 4.
    println!("silêncio → {text:?}");
}

/// Large-v3-turbo em CPU e em cada GPU Vulkan, carregado da memória. Imprime os tempos.
/// Meça em release: o whisper.cpp herda o perfil do Cargo e em debug fica sem otimização.
#[test]
#[ignore = "hardware"]
fn large_cpu_vs_vulkan() {
    let bytes = std::fs::read(model("ggml-large-v3-turbo-q5_0.bin")).unwrap();
    let audio = read_wav_f32(&fixture("fala.wav")).unwrap();
    let secs = audio.len() as f64 / 16_000.0;

    // None = CPU; Some(i) = dispositivo Vulkan i (nesta máquina: 0 Intel, 1 RTX 3050).
    for device in [None, Some(0), Some(1)] {
        let mut p = WhisperContextParameters::default();
        p.use_gpu(device.is_some());
        p.gpu_device(device.unwrap_or(0));
        let t0 = Instant::now();
        let ctx = WhisperContext::new_from_buffer_with_params(&bytes, p).unwrap();
        let load = t0.elapsed();
        let t1 = Instant::now();
        let text = transcribe(&ctx, &audio).unwrap();
        let run = t1.elapsed();
        println!(
            "{}: carga {:.2?}, transcrição {:.2?} ({:.1}x tempo real) → {text}",
            device.map_or("CPU".to_string(), |i| format!("Vulkan {i}")),
            load,
            run,
            secs / run.as_secs_f64()
        );
        assert!(keyword_hits(&text) >= 4, "texto: {text}");
    }
}
