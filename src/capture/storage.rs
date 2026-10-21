//! A gravação em disco: `<data-dir>/gravacoes/<id>/` com `mic.wav`, `sistema.wav` (32 kHz mono
//! 16-bit) e o `gravacao.json`. Fica até a resposta a "Guardar o áudio?"; uma gravação que o
//! app não chegou a transcrever (Ctrl+C, crash) é achada por [`recover`] ao abrir o app.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::align::RATE;
use super::source::Track;
use crate::clock::LocalTime;
use crate::jobs::JobId;

pub const MANIFEST_FILE: &str = "gravacao.json";
pub const MANIFEST_VERSION: u32 = 1;

pub fn wav_spec() -> hound::WavSpec {
    hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecState {
    /// Gravando, ou o app caiu no meio.
    Gravando,
    Parada,
}

/// O `gravacao.json`, versão 1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub id: String,
    /// A hora do Gravar, a mesma do nome da pasta da saída.
    pub created: LocalTime,
    pub mic: String,
    pub output: String,
    pub state: RecState,
}

/// Por que a gravação terminou.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// O usuário clicou em Parar.
    User,
    /// O app está fechando (Ctrl+C): a gravação volta na próxima abertura.
    Exit,
    MicLost,
    OutputLost,
    /// O disco quase encheu.
    Disk,
    /// O teto de duração.
    Limit,
    /// O computador entrou em suspensão (o relógio pulou ou os pacotes sumiram por muito
    /// tempo): o que veio antes vale, sem horas de silêncio no lugar.
    Suspended,
    /// Erro inesperado na thread de escrita.
    Failed,
    /// Achada ao abrir o app.
    Recovered,
}

impl End {
    /// O aviso do fim automático, para o toast; `None` quando o usuário parou.
    pub fn message(self) -> Option<&'static str> {
        Some(match self {
            Self::User | Self::Exit | Self::Recovered => return None,
            Self::MicLost => "O microfone foi desconectado. A gravação parou e vai ser transcrita.",
            Self::OutputLost => {
                "A saída de som foi desconectada. A gravação parou e vai ser transcrita."
            }
            Self::Disk => "O disco está quase cheio. A gravação parou e vai ser transcrita.",
            Self::Limit => "A gravação chegou ao limite de 12 h e parou.",
            Self::Suspended => {
                "O computador entrou em suspensão. A gravação parou e vai ser transcrita."
            }
            Self::Failed => "A gravação parou por um erro. O que foi gravado vai ser transcrito.",
        })
    }
}

/// Como a trilha se comportou: mede a deriva do dispositivo, o loopback no silêncio e o
/// alinhamento. Frames "a 32 kHz" são os do WAV.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackStats {
    /// Frames que o dispositivo entregou, na taxa dele.
    pub received: u64,
    /// A taxa nominal do dispositivo (a do último stream, se a trilha foi reaberta).
    pub rate: u32,
    /// Frames a 32 kHz do Gravar ao 1º pacote: o tempo de abrir o stream.
    pub startup: u64,
    /// Frames a 32 kHz de silêncio no lugar de pacotes que não vieram, depois do 1º.
    pub filled: u64,
    /// Quantas vezes houve esse silêncio.
    pub fills: u32,
    /// Do 1º frame do 1º pacote ao fim do último, no relógio dos pacotes: com `received`,
    /// dá a deriva do relógio do dispositivo.
    pub span_ns: u64,
    /// Frames a 32 kHz descartados por chegarem adiantados.
    pub dropped: u64,
    /// O maior erro, em frames a 32 kHz, entre o fim de um pacote escrito e o relógio.
    pub max_error: u64,
    /// Pacotes descartados por falta de espaço no ring.
    pub lost: u64,
    pub reopens: u32,
}

/// Uma gravação terminada, pronta para virar job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recording {
    pub id: JobId,
    pub dir: PathBuf,
    pub created: LocalTime,
    /// A trilha mais longa (as duas terminam iguais, salvo num crash).
    pub duration_ms: u64,
    /// Os dois WAVs.
    pub size: u64,
    pub mic: String,
    pub output: String,
    pub end: End,
    /// Na ordem de [`Track::ALL`]; zerado numa gravação retomada.
    pub stats: [TrackStats; 2],
}

/// Cria a pasta da gravação `id` dentro de `root`.
pub fn create(root: &Path, id: JobId) -> io::Result<PathBuf> {
    fs::create_dir_all(root)?;
    let dir = root.join(id.to_string());
    fs::create_dir(&dir)?;
    Ok(dir)
}

pub fn write_manifest(dir: &Path, manifest: &Manifest) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;
    crate::transcript::write_atomic(&dir.join(MANIFEST_FILE), &json)
}

pub fn read_manifest(dir: &Path) -> Option<Manifest> {
    let bytes = fs::read(dir.join(MANIFEST_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Frames do WAV da trilha, pelo cabeçalho (atualizado a cada `flush`); `None` se ilegível.
pub fn wav_frames(dir: &Path, track: Track) -> Option<u64> {
    let reader = hound::WavReader::open(dir.join(track.file())).ok()?;
    let spec = reader.spec();
    (spec.sample_rate == RATE && spec.channels == 1).then(|| u64::from(reader.duration()))
}

/// A gravação em `dir` como está no disco.
pub fn load(dir: &Path, end: End) -> Option<Recording> {
    let manifest = read_manifest(dir)?;
    let id = manifest.id.parse().ok()?;
    let frames = Track::ALL
        .iter()
        .filter_map(|&t| wav_frames(dir, t))
        .max()?;
    let size = Track::ALL
        .iter()
        .filter_map(|t| fs::metadata(dir.join(t.file())).ok())
        .map(|m| m.len())
        .sum();
    Some(Recording {
        id,
        dir: dir.to_path_buf(),
        created: manifest.created,
        duration_ms: frames * 1_000 / u64::from(RATE),
        size,
        mic: manifest.mic,
        output: manifest.output,
        end,
        stats: [TrackStats::default(); 2],
    })
}

/// As gravações em `root` que sobraram de uma execução anterior, da mais antiga para a mais
/// nova. As vazias (nada chegou ao disco) ou ilegíveis são apagadas, e os temporários da
/// transcrição (`.f32`) também.
pub fn recover(root: &Path) -> Vec<Recording> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        remove_spools(&dir);
        match load(&dir, End::Recovered) {
            Some(rec) if rec.duration_ms > 0 => found.push(rec),
            _ => {
                eprintln!("gravação vazia ou ilegível apagada: {}", dir.display());
                remove_dir_quietly(&dir);
            }
        }
    }
    found.sort_by_key(|r| r.id);
    found
}

fn remove_spools(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for path in entries.flatten().map(|e| e.path()) {
            if path.extension().is_some_and(|e| e == "f32") {
                let _ = fs::remove_file(path);
            }
        }
    }
}

pub fn remove_dir_quietly(dir: &Path) {
    if let Err(e) = fs::remove_dir_all(dir)
        && e.kind() != io::ErrorKind::NotFound
    {
        eprintln!("não apaguei a gravação {}: {e}", dir.display());
    }
}

/// Bytes livres para o usuário no volume de `path`; `None` se o Windows não informa.
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let path = std::path::absolute(path).ok()?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut available = 0u64;
    // SAFETY: `wide` termina em 0 e vive até o fim da chamada; os ponteiros nulos são
    // permitidos para os totais que não interessam.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: JobId) -> Manifest {
        Manifest {
            version: MANIFEST_VERSION,
            id: id.to_string(),
            created: LocalTime {
                year: 2026,
                month: 10,
                day: 20,
                hour: 14,
                minute: 30,
                second: 5,
                offset_minutes: -180,
            },
            mic: "Microfone".into(),
            output: "Alto-falantes".into(),
            state: RecState::Gravando,
        }
    }

    fn wav(dir: &Path, track: Track, frames: u32) {
        let mut w = hound::WavWriter::create(dir.join(track.file()), wav_spec()).unwrap();
        for i in 0..frames {
            w.write_sample((i % 100) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn manifesto_vai_e_volta() {
        let root = tempfile::tempdir().unwrap();
        let id = JobId::generate();
        let dir = create(root.path(), id).unwrap();
        assert_eq!(dir, root.path().join(id.to_string()));
        write_manifest(&dir, &manifest(id)).unwrap();
        assert_eq!(read_manifest(&dir), Some(manifest(id)));
        let json = fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap();
        assert!(json.contains(r#""state": "gravando""#), "{json}");
        // A mesma pasta não é criada duas vezes.
        assert!(create(root.path(), id).is_err());
    }

    #[test]
    fn retoma_as_gravacoes_e_apaga_as_vazias() {
        let root = tempfile::tempdir().unwrap();
        let (a, b, c) = (JobId::generate(), JobId::generate(), JobId::generate());
        let good = create(root.path(), a).unwrap();
        write_manifest(&good, &manifest(a)).unwrap();
        wav(&good, Track::Mic, 64_000);
        wav(&good, Track::System, 48_000);
        fs::write(good.join("mic.f32"), b"sobra").unwrap();
        // Sem nenhuma amostra: nada a transcrever.
        let empty = create(root.path(), b).unwrap();
        write_manifest(&empty, &manifest(b)).unwrap();
        wav(&empty, Track::Mic, 0);
        // Sem manifesto.
        let broken = create(root.path(), c).unwrap();
        wav(&broken, Track::Mic, 100);

        let found = recover(root.path());
        assert_eq!(found.len(), 1);
        let rec = &found[0];
        assert_eq!(rec.id, a);
        assert_eq!(rec.duration_ms, 2_000);
        assert_eq!(rec.end, End::Recovered);
        assert_eq!(rec.created.hour, 14);
        assert!(rec.size > 2 * 112_000);
        assert!(!good.join("mic.f32").exists());
        assert!(!empty.exists() && !broken.exists());
        assert!(recover(&root.path().join("nada")).is_empty());
    }

    /// O app caiu no meio: o WAV vale até o último `flush`, como a gravação faz a cada 5 s.
    #[test]
    fn crash_depois_do_flush_deixa_a_gravacao_legivel() {
        let root = tempfile::tempdir().unwrap();
        let id = JobId::generate();
        let dir = create(root.path(), id).unwrap();
        write_manifest(&dir, &manifest(id)).unwrap();
        for track in Track::ALL {
            let mut w = hound::WavWriter::create(dir.join(track.file()), wav_spec()).unwrap();
            for i in 0..RATE {
                w.write_sample((i % 1_000) as i16).unwrap();
            }
            w.flush().unwrap();
            // Depois do flush, mais meio segundo que nunca chega ao cabeçalho.
            for _ in 0..RATE / 2 {
                w.write_sample(7i16).unwrap();
            }
            // Sem finalize nem Drop: o processo morreu.
            std::mem::forget(w);
        }
        let found = recover(root.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].duration_ms, 1_000);
        let mut reader = hound::WavReader::open(dir.join(Track::Mic.file())).unwrap();
        let samples: Vec<i16> = reader.samples().map(Result::unwrap).collect();
        assert_eq!(samples.len(), RATE as usize);
        assert_eq!(samples[999], 999);
    }

    #[test]
    fn mensagens_do_fim() {
        assert_eq!(End::User.message(), None);
        assert_eq!(End::Exit.message(), None);
        assert!(End::MicLost.message().unwrap().contains("microfone"));
        assert!(End::Limit.message().unwrap().contains("12 h"));
    }

    #[test]
    fn espaco_livre_do_volume() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(dir.path()).is_some_and(|b| b > 0));
    }
}
