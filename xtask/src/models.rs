use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

pub struct Model {
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

pub const MODELS: &[Model] = &[
    Model {
        file: "ggml-large-v3-turbo-q5_0.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    },
    Model {
        file: "ggml-base-q5_1.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin",
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
    },
    // Candidato a modelo da CPU (benchmark small × base da Fase 4).
    Model {
        file: "ggml-small-q5_1.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin",
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    },
    Model {
        file: "ggml-silero-v6.2.0.bin",
        url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin",
        sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    },
];

/// `only`: textos separados por vírgula; baixa os modelos cujo nome contém algum deles.
pub fn download(dir: &Path, only: Option<&str>) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let selected: Vec<_> = MODELS
        .iter()
        .filter(|m| only.is_none_or(|o| o.split(',').any(|o| m.file.contains(o.trim()))))
        .collect();
    if selected.is_empty() {
        bail!("nenhum modelo corresponde a {only:?}");
    }
    for m in selected {
        fetch(dir, m.file, m.url, m.sha256)?;
    }
    Ok(())
}

/// Baixa `url` para `dir/file`, a menos que já exista com o SHA-256 esperado.
pub fn fetch(dir: &Path, file: &str, url: &str, sha256: &str) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(file);
    if path.exists() && sha256_file(&path)? == sha256 {
        println!("ok   {file}");
        return Ok(());
    }
    println!("baix {file}");
    let tmp = path.with_extension("part");
    let mut body = ureq::get(url).call()?.into_body().into_reader();
    let mut out = File::create(&tmp)?;
    io::copy(&mut body, &mut out).with_context(|| format!("baixando {url}"))?;
    out.flush()?;
    drop(out);
    let got = sha256_file(&tmp)?;
    if got != sha256 {
        fs::remove_file(&tmp)?;
        bail!("SHA-256 de {file} não confere: esperado {sha256}, obtido {got}");
    }
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}
