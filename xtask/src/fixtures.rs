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

/// A conversa das trilhas de gravação (`voce.wav` e `outros.wav`): "Outros" e "Você"
/// alternados, nesta ordem, com 1 s entre as falas. `true` é "Você".
pub const CONVERSA: [(bool, &str); 4] = [
    (
        false,
        "Bom dia a todos. Vamos começar pela revisão do orçamento anual.",
    ),
    (
        true,
        "Obrigado. Eu preparei a planilha com os números do trimestre.",
    ),
    (
        false,
        "Ótimo. Depois falamos do cronograma de entregas do projeto.",
    ),
    (
        true,
        "Combinado. Eu envio o relatório por email amanhã cedo.",
    ),
];

/// O eco no `voce-com-eco.wav`: a trilha "Outros" a −12 dB, 60 ms depois.
const ECHO_DB: f32 = -12.0;
const ECHO_MS: usize = 60;
/// A taxa das trilhas da gravação, a mesma do `Recorder`.
const RECORDING_RATE: u32 = 32_000;

/// Vídeo cinza de 320x240 a 10 fps, a primeira entrada dos vídeos com fala.
const GRAY: [&str; 4] = ["-f", "lavfi", "-i", "color=c=gray:s=320x240:r=10"];
const X264: [&str; 6] = ["-c:v", "libx264", "-preset", "veryfast", "-crf", "35"];

type Make = fn(&Path, &str, &str) -> anyhow::Result<()>;

/// Cada fixture: nome e como gerá-la a partir do `fala.wav` (pasta, wav, saída).
const FIXTURES: [(&str, Make); 13] = [
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
    // Trilhas de gravação (32 kHz mono 16-bit), na ordem: o eco lê as duas primeiras.
    ("voce.wav", |dir, _, o| conversation(dir, true, o)),
    ("outros.wav", |dir, _, o| conversation(dir, false, o)),
    ("voce-com-eco.wav", |dir, _, o| echo(dir, o)),
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

/// Uma trilha da [`CONVERSA`]: as falas de um lado e silêncio do tamanho das do outro, com
/// 0,5 s no começo e no fim. As duas trilhas saem com a mesma duração.
fn conversation(dir: &Path, voce: bool, out: &str) -> anyhow::Result<()> {
    const SPEECH_RATE: u32 = 16_000;
    let tmp = tempdir_in(dir)?;
    let result = (|| {
        let half = vec![0i16; SPEECH_RATE as usize / 2];
        let mut track = half.clone();
        for (i, (is_voce, text)) in CONVERSA.into_iter().enumerate() {
            let wav = tmp.join(format!("fala{i}.wav"));
            speak(text, &wav)?;
            let samples = read_wav(&wav, SPEECH_RATE)?;
            if is_voce == voce {
                track.extend(samples);
            } else {
                track.resize(track.len() + samples.len(), 0);
            }
            if i + 1 < CONVERSA.len() {
                track.resize(track.len() + SPEECH_RATE as usize, 0);
            }
        }
        track.extend(half);
        let at_16k = tmp.join("trilha.wav");
        write_wav(&at_16k, SPEECH_RATE, &track)?;
        ffmpeg(&[
            "-i",
            at_16k.to_str().context("caminho não UTF-8")?,
            "-ar",
            &RECORDING_RATE.to_string(),
            "-c:a",
            "pcm_s16le",
            out,
        ])
    })();
    fs::remove_dir_all(&tmp)?;
    result
}

/// `voce.wav` mais o `outros.wav` atenuado e atrasado: o microfone ouvindo os
/// alto-falantes, sem fone.
fn echo(dir: &Path, out: &str) -> anyhow::Result<()> {
    let voce = read_wav(&dir.join("voce.wav"), RECORDING_RATE)?;
    let outros = read_wav(&dir.join("outros.wav"), RECORDING_RATE)?;
    let gain = 10f32.powf(ECHO_DB / 20.0);
    let delay = RECORDING_RATE as usize * ECHO_MS / 1_000;
    let mixed: Vec<i16> = voce
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let e = i
                .checked_sub(delay)
                .and_then(|j| outros.get(j))
                .map_or(0.0, |&o| f32::from(o) * gain);
            (f32::from(v) + e)
                .round()
                .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
        })
        .collect();
    write_wav(Path::new(out), RECORDING_RATE, &mixed)
}

fn read_wav(path: &Path, rate: u32) -> anyhow::Result<Vec<i16>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("lendo {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_rate != rate || spec.channels != 1 || spec.bits_per_sample != 16 {
        bail!("{} não é {rate} Hz mono 16-bit: {spec:?}", path.display());
    }
    Ok(reader.samples::<i16>().collect::<Result<_, _>>()?)
}

fn write_wav(path: &Path, rate: u32, samples: &[i16]) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    Ok(())
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
