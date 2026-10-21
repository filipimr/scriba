//! `cargo xtask icon`: os `.ico` do exe e da bandeja, rasterizados dos SVGs da marca em
//! `docs/brand/` e gravados em `assets/brand/` (versionados; o `build.rs` os embute).
//!
//! - `scriba.ico`: o glifo branco no quadrado preto arredondado (o `logo.svg`), que aparece
//!   no Explorer claro e no escuro;
//! - `bandeja-escura.ico` e `bandeja-clara.ico`: só o glifo, preto (barra clara) ou branco
//!   (barra escura), sem fundo, como os ícones do sistema na bandeja.
//!
//! Até 24 px vale o desenho simplificado do `logo-16.svg`; acima, o logo completo, com o traço
//! mais grosso até 64 px para não sumir.

use std::fs;
use std::path::Path;

use anyhow::Context;
use resvg::{tiny_skia, usvg};

const APP_SIZES: &[u32] = &[16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
const TRAY_SIZES: &[u32] = &[16, 20, 24, 32, 40, 48];

/// Um ícone: o nome do arquivo e o SVG de cada tamanho.
struct Icon {
    file: &'static str,
    sizes: &'static [u32],
    svg: fn(&Brand, u32) -> String,
}

/// Os SVGs da marca.
struct Brand {
    full: String,
    small: String,
}

const ICONS: &[Icon] = &[
    Icon {
        file: "scriba.ico",
        sizes: APP_SIZES,
        svg: |b, size| {
            if size <= 24 {
                b.small.clone()
            } else {
                thicker(&b.full, size)
            }
        },
    },
    Icon {
        file: "bandeja-escura.ico",
        sizes: TRAY_SIZES,
        svg: |b, size| glyph(b, size, "#000"),
    },
    Icon {
        file: "bandeja-clara.ico",
        sizes: TRAY_SIZES,
        svg: |b, size| glyph(b, size, "#fff"),
    },
];

pub fn run(root: &Path) -> anyhow::Result<()> {
    let brand_dir = root.join("docs").join("brand");
    let read = |name: &str| {
        fs::read_to_string(brand_dir.join(name)).with_context(|| format!("lendo {name}"))
    };
    let brand = Brand {
        full: read("logo.svg")?,
        small: read("logo-16.svg")?,
    };
    let out = root.join("assets").join("brand");
    fs::create_dir_all(&out)?;
    for icon in ICONS {
        let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
        for &size in icon.sizes {
            let rgba = render(&(icon.svg)(&brand, size), size)
                .with_context(|| format!("{} em {size} px", icon.file))?;
            let image = ico::IconImage::from_rgba_data(size, size, rgba);
            dir.add_entry(ico::IconDirEntry::encode(&image)?);
        }
        let path = out.join(icon.file);
        dir.write(fs::File::create(&path)?)
            .with_context(|| format!("gravando {}", path.display()))?;
        println!("{} ({} tamanhos)", path.display(), icon.sizes.len());
    }
    Ok(())
}

/// O traço do logo completo mais grosso nos tamanhos médios.
fn thicker(svg: &str, size: u32) -> String {
    if size <= 64 {
        svg.replace("stroke-width=\"2\"", "stroke-width=\"4\"")
    } else {
        svg.to_string()
    }
}

/// Só o glifo, na cor dada, sem o quadrado de fundo.
fn glyph(brand: &Brand, size: u32, color: &str) -> String {
    let svg = if size <= 24 {
        // Sem o quadrado em volta, o traço fino some na barra.
        brand
            .small
            .replace("stroke-width=\"1.25\"", "stroke-width=\"1.6\"")
    } else {
        thicker(&brand.full, size)
    };
    without_rect(&svg)
        .replace("stroke=\"#fff\"", &format!("stroke=\"{color}\""))
        .replace("stroke=\"#000\"", &format!("stroke=\"{color}\""))
}

fn without_rect(svg: &str) -> String {
    match (
        svg.find("<rect"),
        svg.find("<rect").and_then(|s| svg[s..].find("/>")),
    ) {
        (Some(start), Some(len)) => format!("{}{}", &svg[..start], &svg[start + len + 2..]),
        _ => svg.to_string(),
    }
}

/// Rasteriza o SVG num quadrado de `size` px; devolve RGBA sem pré-multiplicar.
fn render(svg: &str, size: u32) -> anyhow::Result<Vec<u8>> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size).context("tamanho inválido")?;
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brand() -> Brand {
        let dir = crate::root().join("docs").join("brand");
        Brand {
            full: fs::read_to_string(dir.join("logo.svg")).unwrap(),
            small: fs::read_to_string(dir.join("logo-16.svg")).unwrap(),
        }
    }

    /// (cor média dos pixels visíveis, fração visível)
    fn coverage(rgba: &[u8]) -> (u32, f32) {
        let visible: Vec<_> = rgba.chunks(4).filter(|p| p[3] > 128).collect();
        let mean = visible.iter().map(|p| u32::from(p[0])).sum::<u32>() / visible.len() as u32;
        (mean, visible.len() as f32 / (rgba.len() / 4) as f32)
    }

    #[test]
    fn app_e_quadrado_preto_com_glifo_branco() {
        let b = brand();
        for size in [16, 32, 256] {
            let rgba = render(&(ICONS[0].svg)(&b, size), size).unwrap();
            let (mean, visible) = coverage(&rgba);
            assert!(visible > 0.9, "{size}: {visible}");
            assert!(mean < 128, "fundo preto em {size}: {mean}");
        }
    }

    #[test]
    fn bandeja_so_tem_o_glifo_na_cor_do_tema() {
        let b = brand();
        for size in [16, 32] {
            let dark = render(&(ICONS[1].svg)(&b, size), size).unwrap();
            let light = render(&(ICONS[2].svg)(&b, size), size).unwrap();
            let ((dark_mean, dark_vis), (light_mean, light_vis)) =
                (coverage(&dark), coverage(&light));
            assert!(dark_vis > 0.02 && dark_vis < 0.5, "{size}: {dark_vis}");
            assert!((dark_vis - light_vis).abs() < 0.01);
            assert!(
                dark_mean < 40 && light_mean > 215,
                "{dark_mean} {light_mean}"
            );
        }
    }

    #[test]
    fn tira_so_o_quadrado() {
        let svg = r##"<svg><rect width="1" fill="#000"/><path d="M0 0"/></svg>"##;
        assert_eq!(without_rect(svg), r#"<svg><path d="M0 0"/></svg>"#);
        assert_eq!(without_rect("<svg/>"), "<svg/>");
    }
}
