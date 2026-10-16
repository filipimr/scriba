use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

/// Texto lido pela voz Maria. Os testes conferem as palavras-chave.
pub const FALA: &str = "Bom dia. Esta é uma gravação de teste do Scriba. \
    A reunião sobre o orçamento começa amanhã às nove horas.";

pub fn generate(dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let wav = dir.join("fala.wav");
    speak(FALA, &wav)?;

    let w = wav.to_str().context("caminho não UTF-8")?;
    let out = |name: &str| dir.join(name).to_string_lossy().into_owned();
    ffmpeg(&[
        "-i",
        w,
        "-c:a",
        "libmp3lame",
        "-b:a",
        "48k",
        &out("fala.mp3"),
    ])?;
    ffmpeg(&["-i", w, "-c:a", "aac", "-b:a", "48k", &out("fala.m4a")])?;
    ffmpeg(&["-i", w, "-c:a", "libvorbis", "-q:a", "2", &out("fala.ogg")])?;
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=gray:s=320x240:r=10",
        "-i",
        w,
        "-shortest",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "35",
        "-c:a",
        "aac",
        "-b:a",
        "48k",
        &out("fala.mp4"),
    ])?;
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=gray:s=320x240:r=10",
        "-t",
        "2",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "35",
        &out("video-sem-audio.mp4"),
    ])?;
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "anullsrc=r=16000:cl=mono",
        "-t",
        "5",
        "-c:a",
        "pcm_s16le",
        &out("silencio.wav"),
    ])?;
    println!("fixtures em {}", dir.display());
    Ok(())
}

/// Sintetiza `text` com a voz SAPI "Microsoft Maria" em WAV 16 kHz mono 16-bit.
fn speak(text: &str, wav: &Path) -> anyhow::Result<()> {
    let script = format!(
        "Add-Type -AssemblyName System.Speech; \
         $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $v = $s.GetInstalledVoices() | Where-Object {{ $_.VoiceInfo.Name -like '*Maria*' }} | Select-Object -First 1; \
         if (-not $v) {{ throw 'voz Microsoft Maria (pt-BR) não instalada' }}; \
         $s.SelectVoice($v.VoiceInfo.Name); \
         $fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, 'Sixteen', 'Mono'); \
         $s.SetOutputToWaveFile('{}', $fmt); $s.Speak('{}'); $s.Dispose()",
        wav.display(),
        text.replace('\'', "''"),
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()?;
    if !status.success() {
        bail!("síntese de voz falhou");
    }
    Ok(())
}

fn ffmpeg(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .context("ffmpeg não encontrado no PATH (rode scripts/setup.ps1)")?;
    if !status.success() {
        bail!("ffmpeg falhou: {args:?}");
    }
    Ok(())
}
