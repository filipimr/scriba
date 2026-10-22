//! Caminhos do app e configuração persistida em `<data_dir>/config.json`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

    /// A pasta das saídas para quem lê: `Documentos\Scriba` no padrão, senão o caminho.
    pub fn output_label(&self) -> String {
        let in_documents = dirs::document_dir()
            .is_some_and(|docs| self.output_dir.parent() == Some(docs.as_path()));
        match self.output_dir.file_name() {
            Some(name) if in_documents => format!("Documentos\\{}", name.to_string_lossy()),
            _ => self.output_dir.display().to_string(),
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn instance_file(&self) -> PathBuf {
        self.data_dir.join("instance.json")
    }

    /// Cópias dos arquivos enviados, enquanto os jobs não terminam.
    pub fn uploads_dir(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }

    /// As gravações ao vivo, uma pasta por gravação, até a resposta a "Guardar o áudio?".
    pub fn recordings_dir(&self) -> PathBuf {
        self.data_dir.join("gravacoes")
    }

    /// O `scriba.log` e os antigos, com rotação.
    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
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

/// A versão do `config.json` que este Scriba grava. O arquivo sem `version` (antes da Fase 7)
/// é a v0, com as mesmas chaves.
pub const CONFIG_VERSION: u64 = 1;

/// As chaves que o Scriba lê; as outras são preservadas como vieram.
const KNOWN_KEYS: &[&str] = &["version", "theme", "audio_dir", "mic", "output"];

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// A versão lida (ou [`CONFIG_VERSION`]): uma versão mais nova do Scriba continua dona do
    /// arquivo.
    pub version: u64,
    pub theme: Theme,
    /// A última pasta escolhida no "Guardar áudio".
    pub audio_dir: Option<PathBuf>,
    /// O microfone dos Ajustes (id do dispositivo); ausente = o padrão do Windows.
    pub mic: Option<String>,
    /// A saída de som gravada; ausente = o padrão do Windows.
    pub output: Option<String>,
    /// Chaves desconhecidas (de uma versão mais nova), regravadas como vieram.
    pub extra: Map<String, Value>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            theme: Theme::default(),
            audio_dir: None,
            mic: None,
            output: None,
            extra: Map::new(),
        }
    }
}

impl Config {
    /// Arquivo ausente dá a configuração padrão. A leitura é campo a campo: um valor inválido
    /// volta só ele ao padrão. Um JSON corrompido é copiado para `config.json.invalido` (a
    /// próxima gravação o substitui).
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        let Ok(Value::Object(mut map)) = serde_json::from_slice::<Value>(&bytes) else {
            let copy = invalid_copy(path);
            eprintln!(
                "config.json inválido; uma cópia ficou em {} e os ajustes voltaram ao padrão",
                copy.display()
            );
            if let Err(e) = fs::write(&copy, &bytes) {
                eprintln!("não consegui copiar o config.json inválido: {e}");
            }
            return Self::default();
        };
        let mut field = |key: &str| map.remove(key).filter(|v| !v.is_null());
        let text = |v: Option<Value>| match v {
            Some(Value::String(s)) if !s.is_empty() => Some(s),
            _ => None,
        };
        let version = field("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .max(CONFIG_VERSION);
        let theme = field("theme")
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let audio_dir = text(field("audio_dir")).map(PathBuf::from);
        let mic = text(field("mic"));
        let output = text(field("output"));
        for key in KNOWN_KEYS {
            map.remove(*key);
        }
        Self {
            version,
            theme,
            audio_dir,
            mic,
            output,
            extra: map,
        }
    }

    /// Gravação atômica: quem lê vê o arquivo antigo ou o novo inteiro.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut map = self.extra.clone();
        map.insert("version".into(), self.version.into());
        map.insert("theme".into(), serde_json::to_value(self.theme)?);
        let mut put = |key: &str, value: Option<String>| {
            if let Some(value) = value {
                map.insert(key.into(), value.into());
            }
        };
        put(
            "audio_dir",
            self.audio_dir.as_ref().map(|p| p.display().to_string()),
        );
        put("mic", self.mic.clone());
        put("output", self.output.clone());
        let json = serde_json::to_vec_pretty(&Value::Object(map))?;
        crate::transcript::write_atomic(path, &json)
    }
}

/// `config.json.invalido`, ao lado.
fn invalid_copy(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".invalido");
    path.with_file_name(name)
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
        let cfg = Config {
            theme: Theme::Dark,
            audio_dir: Some(r"C:\Áudios\Reuniões".into()),
            mic: Some("{0.0.1.00000000}.{abc}".into()),
            ..Config::default()
        };
        cfg.save(&path).unwrap();
        assert_eq!(Config::load(&path), cfg);
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], 1);
        assert!(saved.get("output").is_none());
        // A gravação atômica não deixa o temporário para trás.
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["config.json"]);
    }

    /// O `config.json` exato que a Fase 6 grava (sem `version`): a v0 migra direto.
    #[test]
    fn migra_a_v0() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let v0 = r#"{
  "theme": "light",
  "audio_dir": "C:\\Users\\Filipi\\Music",
  "mic": "{0.0.1.00000000}.{9b2e}",
  "output": null
}"#;
        fs::write(&path, v0).unwrap();
        let cfg = Config::load(&path);
        assert_eq!(
            cfg,
            Config {
                theme: Theme::Light,
                audio_dir: Some(r"C:\Users\Filipi\Music".into()),
                mic: Some("{0.0.1.00000000}.{9b2e}".into()),
                ..Config::default()
            }
        );
        cfg.save(&path).unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains(r#""version": 1"#)
        );
    }

    #[test]
    fn campo_invalido_volta_so_ele_ao_padrao() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let json = r#"{"version":1,"theme":"roxo","audio_dir":42,"mic":"m","output":["x"]}"#;
        fs::write(&path, json).unwrap();
        assert_eq!(
            Config::load(&path),
            Config {
                mic: Some("m".into()),
                ..Config::default()
            }
        );
        fs::write(&path, "{}").unwrap();
        assert_eq!(Config::load(&path), Config::default());
        fs::write(&path, r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(Config::load(&path).theme, Theme::Dark);
    }

    #[test]
    fn corrompido_fica_copiado_no_invalido() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        for bad in ["{ não é json", "[1, 2]"] {
            fs::write(&path, bad).unwrap();
            assert_eq!(Config::load(&path), Config::default());
            let copy = dir.path().join("config.json.invalido");
            assert_eq!(fs::read_to_string(copy).unwrap(), bad);
        }
        Config::default().save(&path).unwrap();
        assert_eq!(Config::load(&path), Config::default());
    }

    #[test]
    fn chaves_desconhecidas_e_versao_nova_sao_preservadas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let newer = r#"{"version":3,"theme":"dark","iniciar":true,"atalho":{"gravar":"F9"}}"#;
        fs::write(&path, newer).unwrap();
        let mut cfg = Config::load(&path);
        assert_eq!((cfg.version, cfg.theme), (3, Theme::Dark));
        cfg.mic = Some("novo".into());
        cfg.save(&path).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], 3);
        assert_eq!(saved["iniciar"], true);
        assert_eq!(saved["atalho"]["gravar"], "F9");
        assert_eq!(saved["mic"], "novo");
    }

    #[test]
    fn caminhos_explicitos_prevalecem() {
        let p = Paths::resolve(Some("d".into()), Some("o".into()));
        assert_eq!(p.data_dir, PathBuf::from("d"));
        assert_eq!(p.output_dir, PathBuf::from("o"));
        assert_eq!(p.instance_file(), PathBuf::from("d").join("instance.json"));
        assert_eq!(p.recordings_dir(), PathBuf::from("d").join("gravacoes"));
        assert_eq!(p.logs_dir(), PathBuf::from("d").join("logs"));
        assert_eq!(p.output_label(), "o");
        let docs = Paths::resolve(Some("d".into()), None);
        if dirs::document_dir().is_some() {
            assert_eq!(docs.output_label(), r"Documentos\Scriba");
        }
    }
}
