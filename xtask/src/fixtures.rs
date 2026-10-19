use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

/// Texto lido pela voz Maria. Os testes conferem as palavras-chave.
pub const FALA: &str = "Bom dia. Esta é uma gravação de teste do Scriba. \
    A reunião sobre o orçamento começa amanhã às nove horas.";

/// As duas frases do `pausas.wav`, separadas por 3 s de silêncio.
pub const PAUSA_1: &str = "Primeira parte da reunião, sobre o orçamento.";
pub const PAUSA_2: &str = "Segunda parte, depois da pausa, sobre o cronograma.";

/// Vídeo cinza de 320x240 a 10 fps, a primeira entrada dos vídeos com fala.
const GRAY: [&str; 4] = ["-f", "lavfi", "-i", "color=c=gray:s=320x240:r=10"];
const X264: [&str; 6] = ["-c:v", "libx264", "-preset", "veryfast", "-crf", "35"];

type Make = fn(&Path, &str, &str) -> anyhow::Result<()>;

/// Cada fixture: nome e como gerá-la a partir do `fala.wav` (pasta, wav, saída).
const FIXTURES: [(&str, Make); 10] = [
    ("fala.mp3", |_, w, o| {
        ffmpeg(&["-i", w, "-c:a", "libmp3lame", "-b:a", "48k", o])
    }),
    ("fala.m4a", |_, w, o| {
        ffmpeg(&["-i", w, "-c:a", "aac", "-b:a", "48k", o])
    }),
    ("fala.ogg", |_, w, o| {
        ffmpeg(&["-i", w, "-c:a", "libvorbis", "-q:a", "2", o])
    }),
    ("fala.mp4", |_, w, o| {
        video_with_speech(w, &["-c:a", "aac", "-b:a", "48k"], o)
    }),
    ("fala.mkv", |_, w, o| {
        video_with_speech(w, &["-c:a", "libvorbis", "-q:a", "2"], o)
    }),
    ("fala.mov", |_, w, o| {
        video_with_speech(w, &["-c:a", "aac", "-b:a", "48k"], o)
    }),
    ("fala-48k-estereo.m4a", |_, w, o| {
        ffmpeg(&[
            "-i", w, "-ar", "48000", "-ac", "2", "-c:a", "aac", "-b:a", "96k", o,
        ])
    }),
    ("video-sem-audio.mp4", |_, _, o| {
        let mut args = GRAY.to_vec();
        args.extend(["-t", "2"]);
        args.extend(X264);
        args.push(o);
        ffmpeg(&args)
    }),
    ("silencio.wav", |_, _, o| {
        ffmpeg(&[
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=16000:cl=mono",
            "-t",
            "5",
            "-c:a",
            "pcm_s16le",
            o,
        ])
    }),
    ("pausas.wav", |dir, _, o| pauses(dir, o)),
];

/// Gera as fixtures em `dir`; com `only`, só as listadas (nomes separados por vírgula).
/// O `fala.wav` só é sintetizado de novo se pedido ou se ainda não existir, para não mudar
/// as fixtures que os testes já conhecem.
pub fn generate(dir: &Path, only: Option<&str>) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let wanted: Option<Vec<&str>> = only.map(|o| {
        o.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect()
    });
    if let Some(names) = &wanted {
        for name in names {
            if *name != "fala.wav" && !FIXTURES.iter().any(|(n, _)| n == name) {
                bail!("fixture desconhecida: {name}");
            }
        }
    }
    let selected = |name: &str| wanted.as_ref().is_none_or(|w| w.contains(&name));

    let wav = dir.join("fala.wav");
    if selected("fala.wav") || !wav.exists() {
        speak(FALA, &wav)?;
    }
    let w = wav.to_str().context("caminho não UTF-8")?;
    for (name, make) in FIXTURES {
        if !selected(name) {
            continue;
        }
        let out = dir.join(name);
        make(dir, w, out.to_str().context("caminho não UTF-8")?)?;
        println!("{name}");
    }
    println!("fixtures em {}", dir.display());
    Ok(())
}

fn video_with_speech(wav: &str, audio: &[&str], out: &str) -> anyhow::Result<()> {
    // Com `-shortest` e o vídeo infinito, o vídeo sai ~3 s mais longo que o áudio (o
    // `fala.mp4` atual tem 13,7 s de vídeo); aqui o vídeo já nasce com a duração do WAV.
    let duration = format!("{:.3}", wav_secs(Path::new(wav))?);
    let mut args = GRAY.to_vec();
    args.extend(["-i", wav, "-t", &duration]);
    args.extend(X264);
    args.extend(audio);
    args.push(out);
    ffmpeg(&args)
}

/// Duração do WAV de 16 kHz mono 16-bit que a voz gera, pelo tamanho do bloco `data`.
fn wav_secs(path: &Path) -> anyhow::Result<f64> {
    let bytes = fs::read(path)?;
    let at = bytes
        .windows(4)
        .position(|w| w == b"data")
        .context("WAV sem bloco data")?;
    let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into()?);
    Ok(f64::from(len) / 32_000.0)
}

/// Duas frases com 3 s de silêncio no meio: a transcrição deve dar dois parágrafos.
fn pauses(dir: &Path, out: &str) -> anyhow::Result<()> {
    let tmp = tempdir_in(dir)?;
    let (a, b) = (tmp.join("a.wav"), tmp.join("b.wav"));
    speak(PAUSA_1, &a)?;
    speak(PAUSA_2, &b)?;
    let result = ffmpeg(&[
        "-i",
        a.to_str().context("caminho não UTF-8")?,
        "-f",
        "lavfi",
        "-t",
        "3",
        "-i",
        "anullsrc=r=16000:cl=mono",
        "-i",
        b.to_str().context("caminho não UTF-8")?,
        "-filter_complex",
        "[0:a][1:a][2:a]concat=n=3:v=0:a=1",
        "-c:a",
        "pcm_s16le",
        out,
    ]);
    fs::remove_dir_all(&tmp)?;
    result
}

fn tempdir_in(dir: &Path) -> anyhow::Result<std::path::PathBuf> {
    let tmp = dir.join(".tmp-fixtures");
    fs::create_dir_all(&tmp)?;
    Ok(tmp)
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
