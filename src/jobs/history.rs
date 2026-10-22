//! O histórico: ao abrir, as transcrições de `<output-dir>/*/job.json` (manifesto v1) voltam
//! para a lista como jobs concluídos, as [`LIMIT`] mais recentes. Vale a pasta onde o
//! `job.json` foi achado, então uma pasta renomeada continua valendo. Fica de fora, com uma
//! linha no log: pasta sem `job.json`, JSON inválido, versão mais nova e id repetido.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::id::JobId;
use super::model::{Audio, Job, Media, Outputs, State};
use crate::clock::LocalTime;
use crate::transcript::{JOB_FILE, MANIFEST_VERSION, MD_FILE, Manifest, MediaKind, TXT_FILE};

/// Quantas transcrições a lista mostra ao abrir.
pub const LIMIT: usize = 50;

/// O que o histórico achou.
#[derive(Debug, Default)]
pub struct History {
    /// Das mais recentes para as mais antigas, no máximo o limite.
    pub jobs: Vec<Job>,
    /// Há mais transcrições do que as da lista.
    pub older: bool,
    /// Todas as pastas com um `job.json` válido (a limpeza dos `.m4a.parte` passa por elas).
    pub dirs: Vec<PathBuf>,
}

/// Lê as pastas de `output_dir`. Uma pasta que não existe dá o histórico vazio.
pub fn load(output_dir: &Path, limit: usize) -> History {
    let Ok(entries) = fs::read_dir(output_dir) else {
        return History::default();
    };
    let mut found: Vec<(i64, Job)> = Vec::new();
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        match read(&dir) {
            Ok(job) => {
                dirs.push(dir);
                found.push((job.created.unix_seconds(), job));
            }
            Err(why) => eprintln!("histórico: {} ficou de fora: {why}", dir.display()),
        }
    }
    // Da mais recente para a mais antiga; no mesmo segundo, pelo id.
    found.sort_by(|(a, ja), (b, jb)| b.cmp(a).then(jb.id.cmp(&ja.id)));
    let mut seen = HashSet::new();
    let mut jobs = Vec::new();
    let mut older = false;
    for (_, job) in found {
        if !seen.insert(job.id) {
            eprintln!(
                "histórico: {} ficou de fora: id repetido {}",
                output_dir_name(&job),
                job.id
            );
            continue;
        }
        if jobs.len() == limit {
            older = true;
            continue;
        }
        jobs.push(job);
    }
    History { jobs, older, dirs }
}

fn output_dir_name(job: &Job) -> String {
    match &job.state {
        State::Done { outputs, .. } => outputs.dir.display().to_string(),
        _ => job.name.clone(),
    }
}

/// O job concluído de uma pasta, ou o motivo de ela ficar de fora.
fn read(dir: &Path) -> Result<Job, String> {
    let bytes = fs::read(dir.join(JOB_FILE)).map_err(|_| format!("sem {JOB_FILE}"))?;
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("{JOB_FILE} inválido ({e})"))?;
    let version = json.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version > u64::from(MANIFEST_VERSION) {
        return Err(format!("{JOB_FILE} da versão {version}, mais nova"));
    }
    let manifest: Manifest =
        serde_json::from_value(json).map_err(|e| format!("{JOB_FILE} inválido ({e})"))?;
    let id: JobId = manifest
        .id
        .parse()
        .map_err(|_| format!("id inválido {:?}", manifest.id))?;
    let created = LocalTime::parse_rfc3339(&manifest.created)
        .ok_or_else(|| format!("data inválida {:?}", manifest.created))?;
    // Só o nome do arquivo: o manifesto nunca aponta para fora da pasta.
    let file = |name: &str, default: &str| {
        dir.join(
            Path::new(name)
                .file_name()
                .map_or_else(|| default.into(), PathBuf::from),
        )
    };
    let audio = match manifest.audio {
        Some(path) if path.is_file() => Audio::Saved { path },
        _ => Audio::Discarded,
    };
    Ok(Job {
        id,
        name: manifest.name,
        media: match manifest.media {
            MediaKind::Audio => Media::Audio,
            MediaKind::Video => Media::Video,
            MediaKind::Recording => Media::Recording,
        },
        size: manifest.size_bytes,
        duration_ms: manifest.duration_ms,
        created,
        estimate: None,
        // A mídia já foi apagada: o job do histórico não tem.
        input: PathBuf::new(),
        state: State::Done {
            outputs: Outputs {
                txt: file(&manifest.files.txt, TXT_FILE),
                md: file(&manifest.files.md, MD_FILE),
                dir: dir.to_path_buf(),
            },
            audio,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, created: &str) -> String {
        format!(
            r#"{{"version":1,"id":"{id}","name":"a.mp3","media":"audio","size_bytes":1,
            "duration_ms":2,"created":"{created}","backend":"cpu","model":"small",
            "timings":{{"extract_ms":0,"transcribe_ms":0,"total_ms":0}},
            "files":{{"txt":"..\\..\\fora.txt","md":"transcricao.md"}}}}"#
        )
    }

    #[test]
    fn pasta_ausente_da_historico_vazio() {
        let h = load(Path::new("Z:/nao/existe"), LIMIT);
        assert!(h.jobs.is_empty() && !h.older && h.dirs.is_empty());
    }

    #[test]
    fn arquivos_ficam_dentro_da_pasta_e_ordem_pela_hora_com_fuso() {
        let tmp = tempfile::tempdir().unwrap();
        let write = |name: &str, json: String| {
            let dir = tmp.path().join(name);
            fs::create_dir(&dir).unwrap();
            fs::write(dir.join(JOB_FILE), json).unwrap();
        };
        // 21h em Brasília é depois de 23h em Lisboa (+01:00) do mesmo dia.
        write(
            "a",
            manifest("01JA0000000000000000000001", "2026-10-19T21:00:00-03:00"),
        );
        write(
            "b",
            manifest("01JA0000000000000000000002", "2026-10-19T23:00:00+01:00"),
        );
        fs::write(tmp.path().join("solto.txt"), "não é pasta").unwrap();
        let h = load(tmp.path(), LIMIT);
        let names: Vec<_> = h.jobs.iter().map(output_dir_name).collect();
        assert_eq!(
            names,
            [
                tmp.path().join("a").display().to_string(),
                tmp.path().join("b").display().to_string()
            ]
        );
        let State::Done { outputs, .. } = &h.jobs[0].state else {
            panic!()
        };
        assert_eq!(outputs.txt, tmp.path().join("a").join("fora.txt"));
        assert_eq!(h.dirs.len(), 2);
    }
}
