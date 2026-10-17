//! Spike C: mic + loopback WASAPI em duas trilhas, com silêncio tocando na saída.
//!
//! O loopback WASAPI não entrega pacotes enquanto nada toca. O stream de
//! silêncio na mesma saída (técnica do OBS) mantém o relógio andando, então as
//! duas trilhas devem ter a mesma duração mesmo sem som no PC.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

const SECS: u64 = 30;

/// Grava `rx` (amostras f32 intercaladas) como WAV mono 16-bit e devolve a duração em s.
fn writer(
    path: std::path::PathBuf,
    rate: u32,
    channels: u16,
    rx: mpsc::Receiver<Vec<f32>>,
) -> thread::JoinHandle<f64> {
    thread::spawn(move || {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        let mut frames = 0u64;
        for buf in rx {
            for frame in buf.chunks(channels as usize) {
                let mono = frame.iter().sum::<f32>() / frame.len() as f32;
                w.write_sample((mono.clamp(-1.0, 1.0) * 32767.0) as i16)
                    .unwrap();
                frames += 1;
            }
        }
        w.finalize().unwrap();
        frames as f64 / f64::from(rate)
    })
}

fn input(
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
    tx: mpsc::Sender<Vec<f32>>,
) -> Stream {
    assert_eq!(
        config.sample_format(),
        SampleFormat::F32,
        "WASAPI compartilhado entrega f32"
    );
    device
        .build_input_stream(
            config.config(),
            move |data: &[f32], _| {
                let _ = tx.send(data.to_vec());
            },
            |e| eprintln!("erro no stream: {e}"),
            None,
        )
        .unwrap()
}

#[test]
#[ignore = "hardware"]
fn mic_e_loopback_mesma_duracao() {
    let host = cpal::default_host();
    let mic = host.default_input_device().expect("sem microfone");
    let out = host.default_output_device().expect("sem saída de áudio");

    let mic_cfg = mic.default_input_config().unwrap();
    let out_cfg = out.default_output_config().unwrap();
    println!("mic: {mic_cfg:?}\nsaída: {out_cfg:?}");

    let dir = std::env::temp_dir().join("scriba-spike-c");
    std::fs::create_dir_all(&dir).unwrap();

    let (mic_tx, mic_rx) = mpsc::channel();
    let (sys_tx, sys_rx) = mpsc::channel();
    let mic_w = writer(
        dir.join("mic.wav"),
        mic_cfg.sample_rate(),
        mic_cfg.channels(),
        mic_rx,
    );
    let sys_w = writer(
        dir.join("sistema.wav"),
        out_cfg.sample_rate(),
        out_cfg.channels(),
        sys_rx,
    );

    // Silêncio na saída para o loopback nunca parar de entregar pacotes.
    let silence = out
        .build_output_stream(
            out_cfg.config(),
            |data: &mut [f32], _| data.fill(0.0),
            |e| eprintln!("erro no silêncio: {e}"),
            None,
        )
        .unwrap();
    // Loopback: stream de entrada aberto no dispositivo de saída.
    let loopback = input(&out, out_cfg, sys_tx);
    let mic_stream = input(&mic, mic_cfg, mic_tx);

    silence.play().unwrap();
    loopback.play().unwrap();
    mic_stream.play().unwrap();
    println!("gravando {SECS} s; deixe o PC sem som por ~10 s no meio");
    thread::sleep(Duration::from_secs(SECS));
    drop(mic_stream);
    drop(loopback);
    drop(silence);

    let mic_secs = mic_w.join().unwrap();
    let sys_secs = sys_w.join().unwrap();
    println!(
        "mic {mic_secs:.3} s, sistema {sys_secs:.3} s, arquivos em {}",
        dir.display()
    );
    assert!(
        (mic_secs - sys_secs).abs() <= 0.100,
        "diferença acima de 100 ms"
    );
    assert!(
        (mic_secs - SECS as f64).abs() <= 0.5,
        "mic com duração inesperada"
    );
}
