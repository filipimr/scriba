//! Leitura dos tokens do front matter YAML do `DESIGN-scriba.md`.

use anyhow::{Context, bail};
use serde_yaml_ng::{Mapping, Value};

pub struct Design {
    root: Mapping,
}

impl Design {
    /// Extrai e interpreta o bloco entre os dois `---` do início do arquivo.
    pub fn parse(markdown: &str) -> anyhow::Result<Self> {
        let text = markdown.replace("\r\n", "\n");
        let Some(rest) = text.strip_prefix("---\n") else {
            bail!("o DESIGN-scriba.md não começa com front matter (---)");
        };
        let end = rest
            .find("\n---\n")
            .context("front matter sem o --- de fechamento")?;
        let root: Mapping = serde_yaml_ng::from_str(&rest[..end]).context("YAML do design")?;
        Ok(Self { root })
    }

    /// Pares (nome, valor) de um grupo de primeiro nível, na ordem do arquivo.
    pub fn group(&self, name: &str) -> Vec<(String, Value)> {
        entries(self.root.get(name))
    }

    /// Cores do tema claro.
    pub fn light_colors(&self) -> Vec<(String, String)> {
        scalars(self.root.get("colors"))
    }

    /// Só as cores que o tema escuro sobrescreve.
    pub fn dark_overrides(&self) -> Vec<(String, String)> {
        scalars(self.root.get("themes").and_then(|t| t.get("dark")))
    }

    /// Cores efetivas de um tema (o escuro herda do claro o que não sobrescreve).
    pub fn colors(&self, dark: bool) -> Vec<(String, String)> {
        let mut colors = self.light_colors();
        if dark {
            for (name, value) in self.dark_overrides() {
                if let Some(c) = colors.iter_mut().find(|(n, _)| *n == name) {
                    c.1 = value;
                }
            }
        }
        colors
    }
}

fn entries(v: Option<&Value>) -> Vec<(String, Value)> {
    v.and_then(Value::as_mapping)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.clone())))
                .collect()
        })
        .unwrap_or_default()
}

fn scalars(v: Option<&Value>) -> Vec<(String, String)> {
    entries(v)
        .into_iter()
        .filter_map(|(k, v)| Some((k, scalar(&v)?)))
        .collect()
}

/// Texto de um valor escalar do YAML (`17px`, `600`, `0`, `-0.4px`).
pub fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
