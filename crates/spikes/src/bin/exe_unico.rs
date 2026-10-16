//! Spike D: carrega o modelo embutido da memória e transcreve a fixture.
//! Uso: exe_unico <fala.wav> [--gpu <índice>]

use std::time::Instant;

use whisper_rs::{WhisperContext, WhisperContextParameters};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let wav = args
        .get(1)
        .expect("uso: exe_unico <fala.wav> [--gpu <índice>]");
    let gpu = args
        .iter()
        .position(|a| a == "--gpu")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.parse::<i32>())
        .transpose()?;

    let model = scriba_model::WHISPER.expect("compilado sem embedded-model");
    println!("modelo embutido: {:.1} MB", model.len() as f64 / 1e6);

    let mut p = WhisperContextParameters::default();
    p.use_gpu(gpu.is_some());
    p.gpu_device(gpu.unwrap_or(0));
    let t0 = Instant::now();
    let ctx = WhisperContext::new_from_buffer_with_params(model, p)?;
    println!("carga: {:.2?}", t0.elapsed());

    let audio = spikes::read_wav_f32(std::path::Path::new(wav))?;
    let t1 = Instant::now();
    let text = spikes::transcribe(&ctx, &audio)?;
    println!("transcrição: {:.2?} → {text}", t1.elapsed());
    Ok(())
}
