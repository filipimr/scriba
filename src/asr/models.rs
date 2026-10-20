//! Os modelos: o large-v3-turbo para as GPUs, um menor para a CPU e o VAD Silero. O release
//! os embute no exe (`scriba-model`); em dev, vêm de `models/` (`cargo xtask model`).

use std::borrow::Cow;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Arquivos em `models/`, os mesmos do `cargo xtask model`.
pub const LARGE_FILE: &str = "ggml-large-v3-turbo-q5_0.bin";
pub const SMALL_FILE: &str = "ggml-small-q5_1.bin";
pub const BASE_FILE: &str = "ggml-base-q5_1.bin";
pub const VAD_FILE: &str = "ggml-silero-v6.2.0.bin";

/// O modelo da CPU, escolhido pelo benchmark small × base da etapa 4.3.
const CPU_MODEL: (&str, &str) = ("small", SMALL_FILE);

/// O papel do modelo no plano de backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    /// large-v3-turbo, para as GPUs.
    Large,
    /// O modelo menor, para transcrever sem GPU.
    Cpu,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Embedded(&'static [u8]),
    File(PathBuf),
}

impl Source {
    /// O conteúdo inteiro: emprestado do exe ou lido do disco.
    pub fn bytes(&self) -> io::Result<Cow<'static, [u8]>> {
        match self {
            Self::Embedded(bytes) => Ok(Cow::Borrowed(bytes)),
            Self::File(path) => std::fs::read(path).map(Cow::Owned),
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Embedded(bytes) => write!(f, "embutido ({} MB)", bytes.len() >> 20),
            Self::File(path) => write!(f, "{}", path.display()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    /// Nome curto, para o `job.json` e o log: `large-v3-turbo`, `small` ou `base`.
    pub name: &'static str,
    pub source: Source,
}

impl Model {
    fn file(name: &'static str, path: PathBuf) -> Self {
        Self {
            name,
            source: Source::File(path),
        }
    }
}

/// Os modelos disponíveis. Um candidato do plano só entra se o modelo dele está aqui.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Models {
    pub large: Option<Model>,
    pub cpu: Option<Model>,
    pub vad: Option<Source>,
}

impl Models {
    pub fn get(&self, kind: ModelKind) -> Option<&Model> {
        match kind {
            ModelKind::Large => self.large.as_ref(),
            ModelKind::Cpu => self.cpu.as_ref(),
        }
    }

    /// Os embutidos no exe e, para o que faltar, os de `models/` do repositório (dev).
    pub fn locate() -> Self {
        let embedded = Self::embedded();
        let dev = Self::from_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("models"));
        Self {
            large: embedded.large.or(dev.large),
            cpu: embedded.cpu.or(dev.cpu),
            vad: embedded.vad.or(dev.vad),
        }
    }

    /// Os que o build embutiu (`embedded-model`).
    pub fn embedded() -> Self {
        Self {
            large: scriba_model::WHISPER.map(|bytes| Model {
                name: "large-v3-turbo",
                source: Source::Embedded(bytes),
            }),
            cpu: None,
            vad: scriba_model::VAD.map(Source::Embedded),
        }
    }

    /// Os arquivos que existem em `dir`. Sem o modelo da CPU, o base faz o papel dele: o CI
    /// só baixa o base.
    pub fn from_dir(dir: &Path) -> Self {
        let file = |name: &str| Some(dir.join(name)).filter(|p| p.is_file());
        Self {
            large: file(LARGE_FILE).map(|p| Model::file("large-v3-turbo", p)),
            cpu: [CPU_MODEL, ("base", BASE_FILE)]
                .into_iter()
                .find_map(|(name, f)| file(f).map(|p| Model::file(name, p))),
            vad: file(VAD_FILE).map(Source::File),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasta_so_com_o_base_usa_o_base_na_cpu() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Models::from_dir(dir.path()), Models::default());
        for name in [BASE_FILE, VAD_FILE] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let models = Models::from_dir(dir.path());
        assert_eq!(models.large, None);
        assert_eq!(models.get(ModelKind::Cpu).unwrap().name, "base");
        assert_eq!(models.vad, Some(Source::File(dir.path().join(VAD_FILE))));

        std::fs::write(dir.path().join(CPU_MODEL.1), b"x").unwrap();
        std::fs::write(dir.path().join(LARGE_FILE), b"x").unwrap();
        let models = Models::from_dir(dir.path());
        assert_eq!(models.get(ModelKind::Cpu).unwrap().name, CPU_MODEL.0);
        assert_eq!(models.get(ModelKind::Large).unwrap().name, "large-v3-turbo");
    }

    #[test]
    fn le_o_conteudo_de_arquivo_ou_embutido() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vad.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(&*Source::File(path.clone()).bytes().unwrap(), b"abc");
        assert_eq!(&*Source::Embedded(b"xyz").bytes().unwrap(), b"xyz");
        assert!(Source::File(dir.path().join("nada")).bytes().is_err());
        assert_eq!(
            Source::File(path.clone()).to_string(),
            path.display().to_string()
        );
    }
}
