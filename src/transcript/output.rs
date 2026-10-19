//! A gravação da saída: a pasta `AAAA-MM-DD HHmm - nome` com `transcricao.txt`,
//! `transcricao.md` e o manifesto `job.json`, cada um gravado de forma atômica.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Transcript, render_md, render_txt};
use crate::clock::LocalTime;

pub const TXT_FILE: &str = "transcricao.txt";
pub const MD_FILE: &str = "transcricao.md";
pub const JOB_FILE: &str = "job.json";
pub const MANIFEST_VERSION: u32 = 1;

/// Tamanho máximo do nome limpo, em caracteres; a pasta inteira fica bem abaixo do
/// MAX_PATH mesmo dentro de Documentos.
const MAX_NAME: usize = 80;
const FALLBACK_NAME: &str = "Sem nome";

/// Limpa um nome para servir de nome de pasta no Windows: tira caracteres inválidos e de
/// controle, espaços e pontos nas pontas, evita os nomes reservados e corta o tamanho.
pub fn clean_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let joined = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = joined.chars().take(MAX_NAME).collect();
    let mut clean = cut.trim_end_matches([' ', '.']).trim_start().to_string();
    if clean.is_empty() {
        clean = FALLBACK_NAME.to_string();
    }
    if reserved(&clean) {
        clean.insert(0, '_');
    }
    clean
}

/// CON, PRN, AUX, NUL, COM0–9 e LPT0–9 (também com sobrescritos), com ou sem extensão.
fn reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = stem.to_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let rest: Vec<char> = chars.collect();
    matches!(prefix.as_str(), "COM" | "LPT")
        && rest.len() == 1
        && (rest[0].is_ascii_digit() || matches!(rest[0], '¹' | '²' | '³'))
}

/// `2026-10-19 1430 - Reunião`, do nome do arquivo original sem a extensão.
pub fn folder_name(date: &LocalTime, file_name: &str) -> String {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    format!("{} - {}", date.folder_stamp(), clean_name(stem))
}

/// Cria `parent/base`, ou `parent/base (2)`, `(3)`… se já existir. O `create_dir` falha se
/// a pasta existe, então duas gravações ao mesmo tempo nunca dividem a pasta.
pub fn create_unique_dir(parent: &Path, base: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(parent)?;
    for n in 1..10_000 {
        let name = if n == 1 {
            base.to_string()
        } else {
            format!("{base} ({n})")
        };
        let dir = parent.join(name);
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("pastas demais com o nome {base}"),
    ))
}

/// Grava num temporário ao lado e renomeia por cima: quem lê vê o arquivo antigo ou o novo
/// inteiro, nunca pela metade.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "caminho sem nome"))?;
    let tmp = path.with_file_name(format!(".{}.tmp", name.to_string_lossy()));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    #[default]
    Audio,
    Video,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timings {
    pub extract_ms: u64,
    pub transcribe_ms: u64,
    pub total_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Files {
    pub txt: String,
    pub md: String,
}

impl Default for Files {
    fn default() -> Self {
        Self {
            txt: TXT_FILE.to_string(),
            md: MD_FILE.to_string(),
        }
    }
}

/// O `job.json`, versão 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    /// Nome do arquivo original.
    pub name: String,
    pub media: MediaKind,
    pub size_bytes: u64,
    pub duration_ms: u64,
    /// Hora local com o fuso (RFC 3339), a mesma do nome da pasta.
    pub created: String,
    pub backend: String,
    pub model: String,
    pub timings: Timings,
    pub files: Files,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            id: String::new(),
            name: String::new(),
            media: MediaKind::default(),
            size_bytes: 0,
            duration_ms: 0,
            created: String::new(),
            backend: String::new(),
            model: String::new(),
            timings: Timings::default(),
            files: Files::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub dir: PathBuf,
    pub txt: PathBuf,
    pub md: PathBuf,
    pub manifest: PathBuf,
}

/// Cria a pasta do job dentro de `output_dir` e grava os três arquivos. Se algo falha, a
/// pasta é apagada: uma saída pela metade não fica para trás.
pub fn write(output_dir: &Path, t: &Transcript, manifest: &Manifest) -> io::Result<Written> {
    let dir = create_unique_dir(output_dir, &folder_name(&t.header.date, &t.header.file))?;
    let written = Written {
        txt: dir.join(TXT_FILE),
        md: dir.join(MD_FILE),
        manifest: dir.join(JOB_FILE),
        dir,
    };
    let manifest = Manifest {
        version: MANIFEST_VERSION,
        files: Files::default(),
        ..manifest.clone()
    };
    let result = (|| {
        write_atomic(&written.txt, render_txt(t).as_bytes())?;
        write_atomic(&written.md, render_md(t).as_bytes())?;
        let json = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
        write_atomic(&written.manifest, &json)
    })();
    match result {
        Ok(()) => Ok(written),
        Err(e) => {
            let _ = fs::remove_dir_all(&written.dir);
            Err(e)
        }
    }
}
