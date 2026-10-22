//! Contraste WCAG de todo par texto/fundo dos componentes do design, nos dois temas.

use crate::design::{Design, scalar};

/// Fundo efetivo de componentes transparentes ou sem fundo: a superfície onde aparecem.
const CONTEXT_BACKGROUND: &[(&str, &str)] = &[
    ("theme-toggle", "surface-black"),
    ("text-link-on-dark", "surface-dark"),
    ("button-secondary", "canvas"),
    ("text-link", "canvas"),
    ("transcript-timestamp", "canvas"),
    ("transcript-speaker", "canvas"),
    ("empty-state", "canvas-parchment"),
    ("older-note", "canvas-parchment"),
];

pub struct Pair {
    pub component: String,
    pub dark: bool,
    pub text: String,
    pub background: String,
    pub ratio: f64,
    pub minimum: f64,
}

/// Todos os pares texto/fundo, com a razão de contraste e o mínimo AA exigido.
/// Variantes `-disabled` ficam de fora (WCAG 1.4.3 isenta componentes inativos).
pub fn pairs(design: &Design) -> anyhow::Result<Vec<Pair>> {
    let mut out = Vec::new();
    for dark in [false, true] {
        let colors = design.colors(dark);
        let resolve = |reference: &str| -> anyhow::Result<String> {
            let name = reference
                .strip_prefix("{colors.")
                .and_then(|r| r.strip_suffix('}'))
                .unwrap_or(reference);
            colors
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| anyhow::anyhow!("cor desconhecida: {reference}"))
        };
        for (name, spec) in design.group("components") {
            if name.ends_with("-disabled") {
                continue;
            }
            let Some(text) = spec.get("textColor").and_then(scalar) else {
                continue;
            };
            let background = match spec.get("backgroundColor").and_then(scalar) {
                Some(bg) if bg != "transparent" => bg,
                _ => {
                    let ctx = CONTEXT_BACKGROUND
                        .iter()
                        .find(|(c, _)| *c == name)
                        .ok_or_else(|| {
                            anyhow::anyhow!("{name}: sem fundo; declare-o em CONTEXT_BACKGROUND")
                        })?;
                    ctx.1.to_string()
                }
            };
            let (text, background) = (resolve(&text)?, resolve(&background)?);
            out.push(Pair {
                minimum: minimum_for(design, &spec),
                ratio: ratio(&text, &background)?,
                component: name,
                dark,
                text,
                background,
            });
        }
    }
    Ok(out)
}

/// 3:1 para texto grande (≥ 24 px), 4,5:1 para o resto.
fn minimum_for(design: &Design, spec: &serde_yaml_ng::Value) -> f64 {
    let size = spec
        .get("typography")
        .and_then(scalar)
        .and_then(|t| {
            let name = t
                .strip_prefix("{typography.")?
                .strip_suffix('}')?
                .to_string();
            design
                .group("typography")
                .into_iter()
                .find(|(n, _)| *n == name)
        })
        .and_then(|(_, t)| t.get("fontSize").and_then(scalar))
        .and_then(|s| s.trim_end_matches("px").parse::<f64>().ok())
        .unwrap_or(0.0);
    if size >= 24.0 { 3.0 } else { 4.5 }
}

pub fn ratio(a: &str, b: &str) -> anyhow::Result<f64> {
    let (la, lb) = (luminance(a)?, luminance(b)?);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    Ok((hi + 0.05) / (lo + 0.05))
}

fn luminance(hex: &str) -> anyhow::Result<f64> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        anyhow::bail!("cor não é #rrggbb: {hex}");
    }
    let channel = |i: usize| -> anyhow::Result<f64> {
        let c = f64::from(u8::from_str_radix(&h[i..i + 2], 16)?) / 255.0;
        Ok(if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        })
    };
    Ok(0.2126 * channel(0)? + 0.7152 * channel(2)? + 0.0722 * channel(4)?)
}

/// Falha listando todo par abaixo do mínimo.
pub fn check(design: &Design) -> anyhow::Result<()> {
    let failures: Vec<String> = pairs(design)?
        .into_iter()
        .filter(|p| p.ratio < p.minimum)
        .map(|p| {
            format!(
                "{} ({}): {} sobre {} = {:.2}:1, mínimo {}:1",
                p.component,
                if p.dark { "escuro" } else { "claro" },
                p.text,
                p.background,
                p.ratio,
                p.minimum
            )
        })
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("contraste abaixo de AA:\n{}", failures.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_preto_e_branco_e_21() {
        assert!((ratio("#000000", "#ffffff").unwrap() - 21.0).abs() < 1e-9);
        assert!((ratio("#777777", "#777777").unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn todo_texto_dos_componentes_passa_aa_nos_dois_temas() {
        let design = Design::parse(&crate::design_source().unwrap()).unwrap();
        assert!(pairs(&design).unwrap().len() > 40);
        check(&design).unwrap();
    }
}
