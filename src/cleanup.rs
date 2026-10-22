//! A limpeza por idade, na abertura: os temporários do `--self-test` em `%TEMP%` e os
//! `*.m4a.parte` de um "Guardar áudio" interrompido, com mais de [`MAX_AGE`]. O resto
//! (`uploads/`, os `.f32`, as gravações vazias ou transcritas) já sai em outros lugares.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::clock::Clock;

/// Um temporário mais novo pode ser de outra execução, ainda em uso.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// Parte do nome dos temporários do `--self-test` (`<pid>-<n>-scriba-self-test.wav`).
pub const SELF_TEST_MARK: &str = "-scriba-self-test.";
/// O fim do `.m4a` que ainda está sendo gravado. Só o do Scriba: a `audio_dir` pode ser uma
/// pasta como Downloads, com `.parte` de outros programas.
pub const PART_SUFFIX: &str = ".m4a.parte";

/// Apaga os temporários antigos: os do `--self-test` em `temp` e os `.parte` em `audio_dirs`
/// (as pastas do histórico e a última pasta escolhida no "Guardar áudio"). Devolve quantos.
pub fn run(clock: &dyn Clock, temp: &Path, audio_dirs: &[PathBuf]) -> usize {
    let now = clock.now();
    let mut removed = sweep(temp, now, |name| name.contains(SELF_TEST_MARK));
    for dir in audio_dirs {
        removed += sweep(dir, now, |name| name.to_lowercase().ends_with(PART_SUFFIX));
    }
    if removed > 0 {
        eprintln!("limpeza: {removed} temporário(s) antigo(s) apagado(s)");
    }
    removed
}

/// Apaga os arquivos de `dir` (sem descer em subpastas) cujo nome passa em `matches` e que
/// não mudam há [`MAX_AGE`] ou mais.
fn sweep(dir: &Path, now: SystemTime, matches: impl Fn(&str) -> bool) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !matches(&name.to_string_lossy()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age >= MAX_AGE);
        if meta.is_file() && old && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FakeClock;

    #[test]
    fn so_o_que_passou_de_um_dia() {
        let tmp = tempfile::tempdir().unwrap();
        let temp = tmp.path().join("temp");
        let audio = tmp.path().join("audio");
        fs::create_dir_all(&temp).unwrap();
        fs::create_dir_all(&audio).unwrap();
        let files = [
            temp.join("123-0-scriba-self-test.wav"),
            temp.join("outro-programa.wav"),
            audio.join("Reunião.m4a.parte"),
            audio.join("Reunião.m4a"),
            audio.join("download.zip.parte"),
            audio.join("Aula.M4A.parte"),
        ];
        for f in &files {
            fs::write(f, "x").unwrap();
        }
        let clock = FakeClock::new(SystemTime::now());
        let dirs = [audio.clone(), tmp.path().join("sumiu")];
        clock.advance(MAX_AGE - Duration::from_secs(60));
        assert_eq!(run(&clock, &temp, &dirs), 0);
        clock.advance(Duration::from_secs(120));
        assert_eq!(run(&clock, &temp, &dirs), 3);
        let left: Vec<bool> = files.iter().map(|f| f.exists()).collect();
        assert_eq!(left, [false, true, false, true, true, false]);
    }
}
