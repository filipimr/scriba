//! Integração do servidor: rotas, segurança, assets e tema via `tower::oneshot`, e a
//! instância única com o binário real.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use scriba::config::Paths;
use scriba::web::{AppState, router};
use tower::ServiceExt;

const TOKEN: &str = "segredo";
const PORT: u16 = 4321;
const HOST: &str = "127.0.0.1:4321";
const ORIGIN: &str = "http://127.0.0.1:4321";

fn state(dir: &tempfile::TempDir) -> AppState {
    let paths = Paths::resolve(Some(dir.path().into()), Some(dir.path().join("saidas")));
    AppState::new(TOKEN.into(), "inst-1".into(), PORT, paths)
}

fn get(uri: &str) -> axum::http::request::Builder {
    Request::get(uri)
        .header(header::HOST, HOST)
        .header(header::COOKIE, format!("scriba_token={TOKEN}"))
}

async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = router(state.clone()).oneshot(req).await.unwrap();
    let (parts, body) = res.into_parts();
    let bytes = to_bytes(body, usize::MAX).await.unwrap();
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
async fn pagina_inicial_com_cookie() {
    let dir = tempfile::tempdir().unwrap();
    let (status, headers, body) = send(&state(&dir), get("/").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(headers.contains_key(header::CONTENT_SECURITY_POLICY));
    assert!(
        body.contains("<html lang=\"pt-BR\">"),
        "modo Sistema não fixa data-theme"
    );
    assert!(body.contains("Solte um vídeo ou áudio aqui"));
    assert!(body.contains("aria-label=\"Tema: Sistema\""));
}

#[tokio::test]
async fn sem_token_401_e_host_estranho_403() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(&dir);
    let req = Request::get("/")
        .header(header::HOST, HOST)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&s, req).await.0, StatusCode::UNAUTHORIZED);
    let req = Request::get("/assets/app.css")
        .header(header::HOST, HOST)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&s, req).await.0, StatusCode::UNAUTHORIZED);
    let req = Request::get("/")
        .header(header::HOST, "evil.com")
        .header(header::COOKIE, format!("scriba_token={TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&s, req).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn token_na_url_vira_cookie_e_redireciona() {
    let dir = tempfile::tempdir().unwrap();
    let req = Request::get(format!("/?t={TOKEN}"))
        .header(header::HOST, HOST)
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = send(&state(&dir), req).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/");
    let cookie = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.starts_with(&format!("scriba_token={TOKEN};")));
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
}

#[tokio::test]
async fn health_responde_o_id_sem_token() {
    let dir = tempfile::tempdir().unwrap();
    let req = Request::get("/health")
        .header(header::HOST, HOST)
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&state(&dir), req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "inst-1");
}

#[tokio::test]
async fn assets_com_tipo_e_cache() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(&dir);
    let (_, _, page) = send(&s, get("/").body(Body::empty()).unwrap()).await;
    let css_url = page
        .split('"')
        .find(|p| p.starts_with("/assets/app.css?v="))
        .expect("link do CSS versionado");

    let (status, headers, body) = send(&s, get(css_url).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "text/css; charset=utf-8");
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert!(body.contains("--color-primary"));

    let font = "/assets/vendor/InterVariable.woff2";
    let (status, headers, _) = send(&s, get(font).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "font/woff2");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let etag = headers[header::ETAG].clone();
    let req = get(font)
        .header(header::IF_NONE_MATCH, etag)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&s, req).await.0, StatusCode::NOT_MODIFIED);

    let (status, _, _) = send(&s, get("/assets/nada.js").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn alternar_tema_persiste_e_renderiza_no_html() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(&dir);
    let post = || {
        Request::post("/api/theme")
            .header(header::HOST, HOST)
            .header(header::ORIGIN, ORIGIN)
            .header(header::COOKIE, format!("scriba_token={TOKEN}"))
            .body(Body::empty())
            .unwrap()
    };
    let (status, headers, body) = send(&s, post()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["hx-trigger"],
        r#"{"scriba-theme":{"mode":"light"}}"#
    );
    assert!(body.contains("Tema: Claro"));
    let (_, headers, _) = send(&s, post()).await;
    assert_eq!(headers["hx-trigger"], r#"{"scriba-theme":{"mode":"dark"}}"#);

    // Um estado novo lê o config.json salvo: o tema sobrevive a reinícios.
    let (_, _, page) = send(&state(&dir), get("/").body(Body::empty()).unwrap()).await;
    assert!(page.contains(r#"<html lang="pt-BR" data-theme="dark">"#));

    let req = Request::post("/api/theme")
        .header(header::HOST, HOST)
        .header(header::COOKIE, format!("scriba_token={TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(&s, req).await.0,
        StatusCode::FORBIDDEN,
        "POST sem Origin"
    );
}

/// A segunda execução encontra a primeira pelo `instance.json`, imprime a mesma URL e sai.
#[test]
fn segunda_instancia_reaproveita_a_primeira() {
    let dir = tempfile::tempdir().unwrap();
    let exe = env!("CARGO_BIN_EXE_scriba");
    let args = ["--no-open", "--port", "0", "--data-dir"];
    let mut first = Command::new(exe)
        .args(args)
        .arg(dir.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(first.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let url = line
        .trim()
        .strip_prefix("Scriba em ")
        .expect(&line)
        .to_string();

    let second = Command::new(exe)
        .args(args)
        .arg(dir.path())
        .output()
        .unwrap();
    first.kill().unwrap();
    first.wait().unwrap();

    assert!(second.status.success());
    let out = String::from_utf8_lossy(&second.stdout);
    assert_eq!(out.trim(), format!("Scriba já está aberto em {url}"));
}
