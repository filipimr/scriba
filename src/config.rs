//! Caminhos do app e configuração persistida em `<data_dir>/config.json`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Onde o app guarda dados internos e onde entrega as saídas.
#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub output_dir: PathBuf,
}

impl Paths {
    /// Padrões: `%LOCALAPPDATA%\Scriba` e `Documentos\Scriba`.
    pub fn resolve(data_dir: Option<PathBuf>, output_dir: Option<PathBuf>) -> Self {
        let data_dir = data_dir.unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("Scriba")
        });
        let output_dir = output_dir.unwrap_or_else(|| {
            dirs::document_dir()
                .unwrap_or_else(|| data_dir.clone())
                .join("Scriba")
        });
        Self {
            data_dir,
            output_dir,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn instance_file(&self) -> PathBuf {
        self.data_dir.join("instance.json")
    }
}

/// Modo de tema escolhido no header. `System` segue o Windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    /// Ciclo do botão: Sistema → Claro → Escuro → Sistema.
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    /// Valor de `data-theme` no `<html>`; `None` no modo Sistema.
    pub fn attr(self) -> Option<&'static str> {
        match self {
            Self::System => None,
            Self::Light => Some("light"),
            Self::Dark => Some("dark"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Sistema",
            Self::Light => "Claro",
            Self::Dark => "Escuro",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub theme: Theme,
}

impl Config {
    /// Arquivo ausente ou inválido dá a configuração padrão.
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciclo_de_tema() {
        assert_eq!(Theme::System.next(), Theme::Light);
        assert_eq!(Theme::Light.next(), Theme::Dark);
        assert_eq!(Theme::Dark.next(), Theme::System);
        assert_eq!(Theme::System.attr(), None);
        assert_eq!(Theme::Dark.attr(), Some("dark"));
    }

    #[test]
    fn config_salva_e_carrega() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.json");
        assert_eq!(Config::load(&path), Config::default());
        let cfg = Config { theme: Theme::Dark };
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path), cfg);
    }

    #[test]
    fn config_invalida_ou_parcial_usa_padrao() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{ não é json").unwrap();
        assert_eq!(Config::load(&path), Config::default());
        fs::write(&path, "{}").unwrap();
        assert_eq!(Config::load(&path).theme, Theme::System);
    }

    #[test]
    fn caminhos_explicitos_prevalecem() {
        let p = Paths::resolve(Some("d".into()), Some("o".into()));
        assert_eq!(p.data_dir, PathBuf::from("d"));
        assert_eq!(p.output_dir, PathBuf::from("o"));
        assert_eq!(p.instance_file(), PathBuf::from("d").join("instance.json"));
    }
}
