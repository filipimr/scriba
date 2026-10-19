//! Integração do servidor: rotas, segurança, assets, tema e fila de jobs via
//! `tower::oneshot`, e a instância única com o binário real.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use scriba::config::Paths;
use scriba::jobs::Jobs;
use scriba::web::{AppState, router};
use tower::ServiceExt;

const TOKEN: &str = "segredo";
const PORT: u16 = 4321;
const HOST: &str = "127.0.0.1:4321";
const ORIGIN: &str = "http://127.0.0.1:4321";

/// Estado com a fila sem worker: os jobs ficam "Na fila" até serem cancelados.
fn state(dir: &tempfile::TempDir) -> AppState {
    let paths = Paths::resolve(Some(dir.path().into()), Some(dir.path().join("saidas")));
    let jobs = Jobs::without_worker(paths.uploads_dir()).unwrap();
    AppState::new(TOKEN.into(), "inst-1".into(), PORT, paths, jobs)
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

// Fila de jobs -----------------------------------------------------------------------

const BOUNDARY: &str = "scriba-teste";

type Part<'a> = (&'a str, Option<&'a str>, &'a [u8]);

const TRANSCREVER: Part = ("transcrever", None, b"1");

/// Corpo multipart com os campos na ordem dada; `Some(nome)` marca um campo de arquivo.
fn multipart(parts: &[Part]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, file, data) in parts {
        body.extend(
            format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"").bytes(),
        );
        if let Some(file) = file {
            body.extend(
                format!("; filename=\"{file}\"\r\nContent-Type: application/octet-stream").bytes(),
            );
        }
        body.extend(b"\r\n\r\n");
        body.extend(*data);
        body.extend(b"\r\n");
    }
    body.extend(format!("--{BOUNDARY}--\r\n").bytes());
    body
}

fn post_jobs(parts: &[Part]) -> Request<Body> {
    Request::post("/api/jobs")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, format!("scriba_token={TOKEN}"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart(parts)))
        .unwrap()
}

fn with_origin(builder: axum::http::request::Builder) -> Request<Body> {
    builder
        .header(header::HOST, HOST)
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, format!("scriba_token={TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

/// Nome e tamanho dos arquivos em `uploads/`.
fn uploads(dir: &tempfile::TempDir) -> Vec<(String, u64)> {
    let mut files: Vec<(String, u64)> = std::fs::read_dir(dir.path().join("uploads"))
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            (name, e.metadata().unwrap().len())
        })
        .collect();
    files.sort();
    files
}

/// Ids dos cards, na ordem em que aparecem no HTML.
fn card_ids(html: &str) -> Vec<String> {
    html.split("<article id=\"job-")
        .skip(1)
        .map(|s| s[..26].to_string())
        .collect()
}

#[tokio::test]
async fn envio_cria_job_e_grava_o_arquivo() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(&dir);
    // Acima dos 2 MB padrão do Axum: o limite não vale na rota de envio.
    let data = vec![7u8; 3 * 1024 * 1024];
    let file = ("arquivo", Some("Reunião.MP4"), data.as_slice());
    let (status, _, body) = send(&s, post_jobs(&[TRANSCREVER, file])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("Reunião.MP4") && body.contains("Na fila"),
        "{body}"
    );
    assert!(body.contains("3 MB"));
    assert!(
        body.contains(r#"hx-trigger="every 1s""#),
        "card ativo faz polling"
    );
    assert!(
        body.contains(r#"<div id="avisos" hx-swap-oob="innerHTML">Reunião.MP4 na fila.</div>"#)
    );
    let ids = card_ids(&body);
    assert_eq!(ids.len(), 1);
    assert_eq!(
        uploads(&dir),
        [(format!("{}.mp4", ids[0]), data.len() as u64)]
    );

    let (status, _, list) = send(&s, get("/api/jobs").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(card_ids(&list), ids);
    assert!(!list.contains("hx-swap-oob"), "a lista não traz aviso");
    let (_, _, page) = send(&s, get("/").body(Body::empty()).unwrap()).await;
    assert_eq!(card_ids(&page), ids);
}

#[tokio::test]
async fn envio_invalido_e_recusado_sem_sobrar_arquivo() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(&dir);
    let formato = "Formato não suportado. Use MP4, MOV, MKV, MP3, M4A, WAV ou OGG.";
    let cases: Vec<(Vec<Part>, &str)> = vec![
        (
            vec![TRANSCREVER, ("arquivo", Some("nota.pdf"), b"x")],
            formato,
        ),
        (
            vec![("arquivo", Some("a.wav"), b"x")],
            "Escolha Transcrever, Compactar ou os dois.",
        ),
        (
            vec![TRANSCREVER, ("arquivo", Some(""), b"")],
            "Escolha um arquivo para enviar.",
        ),
        // O segundo arquivo é inválido: o primeiro, já gravado, também é descartado.
        (
            vec![
                TRANSCREVER,
                ("arquivo", Some("a.wav"), b"x"),
                ("arquivo", Some("b.exe"), b"x"),
            ],
            formato,
        ),
    ];
    for (parts, message) in cases {
        let (status, headers, body) = send(&s, post_jobs(&parts)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(headers["hx-retarget"], "#erro-envio");
        assert_eq!(headers["hx-reswap"], "innerHTML");
        assert!(
            body.contains(message) && body.contains(r#"role="alert""#),
            "{body}"
        );
        assert!(uploads(&dir).is_empty(), "{message}");
    }
    let (_, _, list) = send(&s, get("/api/jobs").body(Body::empty()).unwrap()).await;
    assert!(card_ids(&list).is_empty());
}

#[tokio::test]
async fn varios_arquivos_viram_varios_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(&dir);
    let parts = [
        ("compactar", None, b"1".as_slice()),
        ("arquivo", Some("a.wav"), b"aa"),
        ("arquivo", Some("b.mp3"), b"bbb"),
    ];
    let (status, _, body) = send(&s, post_jobs(&parts)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids = card_ids(&body);
    assert_eq!(ids.len(), 2);
    assert!(ids[0] > ids[1], "o mais recente primeiro");
    assert!(body.find("b.mp3") < body.find("a.wav"));
    assert!(body.contains("2 arquivos na fila."));
    assert_eq!(uploads(&dir).len(), 2);
}

#[tokio::test]
async fn envio_sem_origin_e_proibido() {
    let dir = tempfile::tempdir().unwrap();
    let mut req = post_jobs(&[TRANSCREVER, ("arquivo", Some("a.wav"), b"x")]);
    req.headers_mut().remove(header::ORIGIN);
    assert_eq!(send(&state(&dir), req).await.0, StatusCode::FORBIDDEN);
    assert!(uploads(&dir).is_empty());
}

#[tokio::test]
async fn polling_cancelar_e_remover() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(&dir);
    let file = ("arquivo", Some("a.wav"), b"x".as_slice());
    let (_, _, body) = send(&s, post_jobs(&[TRANSCREVER, file])).await;
    let id = card_ids(&body).remove(0);
    let job = format!("/api/jobs/{id}");

    let (status, _, card) = send(&s, get(&job).body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(card.contains(r#"hx-trigger="every 1s""#) && card.contains("Cancelar a.wav"));
    assert!(!card.contains("hx-swap-oob"));
    let (status, _, _) = send(&s, with_origin(Request::delete(&job))).await;
    assert_eq!(status, StatusCode::CONFLICT, "job ativo não é removido");

    let cancel = format!("{job}/cancel");
    let (status, _, card) = send(&s, with_origin(Request::post(&cancel))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        card.contains("Cancelado") && card.contains("Remover a.wav"),
        "{card}"
    );
    assert!(!card.contains("hx-trigger"), "card final não faz polling");
    assert!(card.contains("a.wav: cancelado."));
    assert!(uploads(&dir).is_empty(), "cancelado na fila apaga o envio");

    let (status, _, card) = send(&s, get(&job).body(Body::empty()).unwrap()).await;
    assert_eq!(status.as_u16(), 286, "fim do polling");
    assert!(card.contains("a.wav: cancelado."));
    let (status, _, card) = send(&s, with_origin(Request::post(&cancel))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !card.contains("hx-swap-oob"),
        "cancelar de novo não muda nada"
    );

    let (status, _, body) = send(&s, with_origin(Request::delete(&job))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty());
    for req in [
        get(&job).body(Body::empty()).unwrap(),
        with_origin(Request::delete(&job)),
        with_origin(Request::post(&cancel)),
        get("/api/jobs/nao-e-um-id").body(Body::empty()).unwrap(),
    ] {
        assert_eq!(send(&s, req).await.0, StatusCode::NOT_FOUND);
    }
}
