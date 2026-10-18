//! `cargo xtask css`: gera `ui/tokens.css` a partir do design e compila `assets/app.css`
//! com o Tailwind CLI standalone (versão fixa, conferida por SHA-256).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};

use crate::design::{Design, scalar};
use crate::{contrast, models};

const TAILWIND_VERSION: &str = "4.3.3";
const TAILWIND_URL: &str = "https://github.com/tailwindlabs/tailwindcss/releases/download/v4.3.3/tailwindcss-windows-x64.exe";
const TAILWIND_SHA256: &str = "e0e260ce048014e9268f6237ff18f8ccf02cef521cbd0ae04e82c2cdf7aa3955";

pub fn run(root: &Path, check: bool) -> anyhow::Result<()> {
    let design = Design::parse(&fs::read_to_string(root.join("DESIGN-scriba.md"))?)?;
    contrast::check(&design)?;
    let tokens = tokens_css(&design);
    let tailwind = tailwind(root)?;

    let tokens_path = root.join("ui").join("tokens.css");
    let css_path = root.join("assets").join("app.css");
    if !check {
        fs::write(&tokens_path, &tokens)?;
        compile(&tailwind, root, &css_path)?;
        println!("ok   {}", css_path.display());
        return Ok(());
    }

    // --check: compila numa cópia e compara com o que está versionado.
    let committed_tokens = read_normalized(&tokens_path)?;
    if committed_tokens != tokens {
        bail!("ui/tokens.css desatualizado: rode `cargo xtask css`");
    }
    let tmp = std::env::temp_dir().join(format!("scriba-app-{}.css", std::process::id()));
    compile(&tailwind, root, &tmp)?;
    let fresh = read_normalized(&tmp)?;
    fs::remove_file(&tmp).ok();
    if read_normalized(&css_path)? != fresh {
        bail!("assets/app.css desatualizado: rode `cargo xtask css`");
    }
    println!("ok   CSS em dia");
    Ok(())
}

fn read_normalized(path: &Path) -> anyhow::Result<String> {
    Ok(fs::read_to_string(path)
        .with_context(|| format!("lendo {}", path.display()))?
        .replace("\r\n", "\n"))
}

fn compile(tailwind: &Path, root: &Path, out: &Path) -> anyhow::Result<()> {
    let status = Command::new(tailwind)
        .current_dir(root)
        .args(["-i", "ui/app.css", "-o"])
        .arg(out)
        .arg("--minify")
        .status()
        .context("executando o Tailwind CLI")?;
    if !status.success() {
        bail!("o Tailwind CLI falhou ({status})");
    }
    Ok(())
}

/// Baixa (uma vez) o Tailwind CLI para `<target>/tools/`.
fn tailwind(root: &Path) -> anyhow::Result<PathBuf> {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let dir = target.join("tools");
    let file = format!("tailwindcss-{TAILWIND_VERSION}.exe");
    models::fetch(&dir, &file, TAILWIND_URL, TAILWIND_SHA256)?;
    Ok(dir.join(file))
}

/// Tokens do design como variáveis CSS no `@theme` do Tailwind (tema claro, que vai para
/// `:root`), mais as sobrescritas do tema escuro: fixo com `data-theme="dark"` e, no modo
/// Sistema (sem atributo), via `prefers-color-scheme`.
pub fn tokens_css(design: &Design) -> String {
    let mut s = String::from(
        "/* Gerado por `cargo xtask css` a partir do DESIGN-scriba.md. Não editar. */\n\n@theme static {\n  --color-*: initial;\n",
    );
    for (name, value) in design.light_colors() {
        let _ = writeln!(s, "  --color-{name}: {value};");
    }

    let typography = design.group("typography");
    if let Some(family) = typography
        .first()
        .and_then(|(_, t)| t.get("fontFamily").and_then(scalar))
    {
        let _ = writeln!(s, "  --font-sans: {family};");
    }
    for (name, t) in &typography {
        let get = |k: &str| t.get(k).and_then(scalar);
        if let Some(size) = get("fontSize") {
            let _ = writeln!(s, "  --text-{name}: {size};");
        }
        for (key, suffix) in [
            ("lineHeight", "line-height"),
            ("letterSpacing", "letter-spacing"),
            ("fontWeight", "font-weight"),
        ] {
            if let Some(v) = get(key) {
                let _ = writeln!(s, "  --text-{name}--{suffix}: {v};");
            }
        }
    }

    for (group, prefix) in [
        ("rounded", "radius"),
        ("spacing", "spacing"),
        ("motion", "motion"),
    ] {
        for (name, v) in design.group(group) {
            if let Some(v) = scalar(&v) {
                let _ = writeln!(s, "  --{prefix}-{name}: {v};");
            }
        }
    }
    for (name, v) in design.group("elevation") {
        if let Some(v) = scalar(&v) {
            let prefix = if name == "floating" {
                "shadow"
            } else {
                "effect"
            };
            let _ = writeln!(s, "  --{prefix}-{name}: {v};");
        }
    }
    s.push_str("}\n\n");

    let mut dark = String::new();
    for (name, value) in design.dark_overrides() {
        let _ = writeln!(dark, "    --color-{name}: {value};");
    }
    dark.push_str("    color-scheme: dark;\n");
    let _ = write!(
        s,
        "@layer base {{\n  :root {{\n    color-scheme: light;\n  }}\n  [data-theme=\"dark\"] {{\n{dark}  }}\n  @media (prefers-color-scheme: dark) {{\n    :root:not([data-theme=\"light\"]) {{\n{}    }}\n  }}\n}}\n",
        dark.replace("    ", "      ")
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_css_snapshot() {
        let design = Design::parse(&crate::design_source().unwrap()).unwrap();
        insta::assert_snapshot!(tokens_css(&design));
    }

    #[test]
    fn tema_escuro_so_sobrescreve_o_que_difere() {
        let design = Design::parse(
            "---\ncolors:\n  ink: \"#111111\"\n  canvas: \"#ffffff\"\nthemes:\n  dark:\n    ink: \"#eeeeee\"\n---\n",
        )
        .unwrap();
        let css = tokens_css(&design);
        assert!(css.contains("--color-ink: #111111;"));
        assert!(css.contains("[data-theme=\"dark\"] {\n    --color-ink: #eeeeee;"));
        assert!(!css.contains("--color-canvas: #ffffff;\n    color-scheme: dark"));
        assert_eq!(css.matches("--color-canvas").count(), 1);
    }
}
