//! Assets embutidos no binário. URLs com `?v=<hash>` recebem cache imutável; sem a
//! versão (ex.: a fonte, referenciada pelo CSS), o navegador revalida pelo ETag.

use std::sync::LazyLock;

use axum::extract::{Path, RawQuery};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

struct Asset {
    path: &'static str,
    mime: &'static str,
    bytes: &'static [u8],
}

macro_rules! asset {
    ($path:literal, $mime:literal) => {
        Asset {
            path: $path,
            mime: $mime,
            bytes: include_bytes!(concat!("../../assets/", $path)),
        }
    };
}

static ASSETS: &[Asset] = &[
    asset!("app.css", "text/css; charset=utf-8"),
    asset!("app.js", "text/javascript; charset=utf-8"),
    asset!("vendor/htmx.min.js", "text/javascript; charset=utf-8"),
    asset!("vendor/InterVariable.woff2", "font/woff2"),
    asset!("brand/logo-white.svg", "image/svg+xml"),
    asset!("brand/logo-16.svg", "image/svg+xml"),
];

/// Hash FNV-1a de cada asset, calculado uma vez.
static HASHES: LazyLock<Vec<String>> = LazyLock::new(|| {
    ASSETS
        .iter()
        .map(|a| {
            let h = a.bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
            });
            format!("{h:016x}")
        })
        .collect()
});

fn find(path: &str) -> Option<(&'static Asset, &'static str)> {
    let i = ASSETS.iter().position(|a| a.path == path)?;
    Some((&ASSETS[i], HASHES[i].as_str()))
}

/// URL versionada de um asset, para os templates.
pub fn url(path: &str) -> String {
    let (_, hash) = find(path).unwrap_or_else(|| panic!("asset inexistente: {path}"));
    format!("/assets/{path}?v={hash}")
}

pub async fn serve(
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some((asset, hash)) = find(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let etag = format!("\"{hash}\"");
    let versioned = query.as_deref() == Some(&format!("v={hash}"));
    let cache = if versioned {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|v| v.as_bytes() == etag.as_bytes());
    let common = [
        (header::ETAG, etag),
        (header::CACHE_CONTROL, cache.to_string()),
    ];
    if not_modified {
        return (StatusCode::NOT_MODIFIED, common).into_response();
    }
    (common, [(header::CONTENT_TYPE, asset.mime)], asset.bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_tem_versao_estavel() {
        let u = url("app.css");
        assert!(u.starts_with("/assets/app.css?v="));
        assert_eq!(u, url("app.css"));
        assert_ne!(u, url("app.js"));
    }

    #[test]
    #[should_panic(expected = "asset inexistente")]
    fn url_de_asset_inexistente_e_bug() {
        url("nao-existe.css");
    }
}
