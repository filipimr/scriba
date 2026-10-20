//! Integração do servidor: rotas, segurança, assets, tema e fila de jobs via
//! `tower::oneshot`, e a instância única com o binário real.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use scriba::asr::{Asr, Models};
use scriba::config::{Config, Paths};
use scriba::dialog::{SaveDialog, SaveRequest};
use scriba::engine::Engine;
use scriba::jobs::{Ctx, Estimate, Failure, Job, Jobs, Media, Outputs, Pipeline, Probe};
use scriba::opener::Opener;
use scriba::transcript::{self, Header, Segment, Transcript};
use scriba::web::{AppState, router};
use tower::ServiceExt;

mod common;

const TOKEN: &str = "segredo";
const PORT: u16 = 4321;
const HOST: &str = "127.0.0.1:4321";
const ORIGIN: &str = "http://127.0.0.1:4321";

/// Probe pelo nome (`sem-audio` é recusado) e, no worker, `falha` falha; o resto conclui na
/// hora com a pasta da saída criada em `out`, com o `.txt` e o `.md` (`sem-fala`: sem
/// parágrafos).
struct WebPipeline {
    out: std::path::PathBuf,
}

impl Pipeline for WebPipeline {
    fn probe(&self, name: &str, _: &Path) -> Result<Probe, Failure> {
        if name.contains("sem-audio") {
            return Err(Failure::Error(
                "Este arquivo não tem áudio. Escolha um vídeo com som.".into(),
            ));
        }
        Ok(Probe {
            media: Media::detect(name).unwrap().0,
            duration_ms: 125_000,
            estimate: Some(Estimate {
                secs: 240,
                gpu: false,
            }),
        })
    }

    fn backend(&self) -> Option<String> {
        Some("Transcrição no processador".into())
    }

    fn run(&self, job: &Job, _: &Ctx) -> Result<Outputs, Failure> {
        if job.name.contains("falha") {
            return Err(Failure::Error("Falhou de propósito.".into()));
        }
        let dir = self.out.join(format!("pasta {}", job.name));
        std::fs::create_dir_all(&dir).unwrap();
        let segments: Vec<Segment> = SPEECH
            .iter()
            .filter(|_| !job.name.contains("sem-fala"))
            .map(|&(start_ms, end_ms, text)| Segment {
                start_ms,
                end_ms,
                text: text.into(),
            })
            .collect();
        let header = Header {
            file: job.name.clone(),
            date: job.created,
            duration_ms: job.duration_ms,
        };
        let t = Transcript::new(header, &segments);
        let outputs = Outputs {
            txt: dir.join(transcript::TXT_FILE),
            md: dir.join(transcript::MD_FILE),
            dir,
        };
        std::fs::write(&outputs.txt, transcript::render_txt(&t)).unwrap();
        std::fs::write(&outputs.md, transcript::render_md(&t)).unwrap();
        Ok(outputs)
    }

    /// Grava "áudio de <nome>"; `audio-ruim` falha e `audio-sem-fim` grava até ser
    /// cancelado.
    fn save_audio(
        &self,
        job: &Job,
        output: &Path,
        progress: &mut dyn FnMut(u8),
        cancel: &AtomicBool,
    ) -> Result<(), Failure> {
        if job.name.contains("audio-ruim") {
            return Err(Failure::Error("Não consegui guardar o áudio.".into()));
        }
        progress(10);
        while job.name.contains("audio-sem-fim") {
            if cancel.load(Ordering::Relaxed) {
                return Err(Failure::Canceled);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::write(output, format!("áudio de {}", job.name)).unwrap();
        Ok(())
    }
}

/// O "Salvar como" de teste: responde `answer` e registra os pedidos.
#[derive(Default)]
struct Answers {
    answer: Mutex<Option<PathBuf>>,
    asked: Mutex<Vec<SaveRequest>>,
}

impl SaveDialog for Answers {
    fn ask(&self, request: &SaveRequest) -> std::io::Result<Option<PathBuf>> {
        self.asked.lock().unwrap().push(request.clone());
        Ok(self.answer.lock().unwrap().clone())
    }
}

/// Os dois parágrafos que o `WebPipeline` grava.
const SPEECH: [(u64, u64, &str); 2] = [
    (0, 3_000, "Bom dia & <boa> reunião."),
    (9_000, 12_000, "Segundo parágrafo."),
];

/// "Abrir pasta" e "Mostrar no Explorer" que só registram os caminhos; com `fail`, devolvem
/// erro.
#[derive(Default)]
struct Recording {
    opened: Mutex<Vec<PathBuf>>,
    revealed: Mutex<Vec<PathBuf>>,
    fail: AtomicBool,
}

impl Opener for Recording {
    fn open(&self, dir: &Path) -> std::io::Result<()> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("sem Explorer"));
        }
        self.opened.lock().unwrap().push(dir.to_path_buf());
        Ok(())
    }

    fn reveal(&self, file: &Path) -> std::io::Result<()> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("sem Explorer"));
        }
        self.revealed.lock().unwrap().push(file.to_path_buf());
        Ok(())
    }
}

fn paths(dir: &tempfile::TempDir) -> Paths {
    Paths::resolve(Some(dir.path().into()), Some(dir.path().join("saidas")))
}

fn pipeline(paths: &Paths) -> Arc<WebPipeline> {
    Arc::new(WebPipeline {
        out: paths.output_dir.clone(),
    })
}

/// Estado com a fila sem worker: os jobs ficam "Na fila" até serem cancelados.
fn state(dir: &tempfile::TempDir) -> AppState {
    let paths = paths(dir);
    let jobs = Jobs::without_worker(paths.uploads_dir(), pipeline(&paths)).unwrap();
    app_state(paths, jobs, Arc::default())
}

fn app_state(paths: Paths, jobs: Jobs, opener: Arc<Recording>) -> AppState {
    dialog_state(paths, jobs, opener, Arc::default())
}

fn dialog_state(
    paths: Paths,
    jobs: Jobs,
    opener: Arc<Recording>,
    dialog: Arc<Answers>,
) -> AppState {
    AppState::new(
        TOKEN.into(),
        "inst-1".into(),
        PORT,
        paths,
        jobs,
        opener,
        dialog,
    )
}

/// Com o worker, o opener que registra e o "Salvar como" de teste.
fn audio_state(
    dir: &tempfile::TempDir,
) -> (AppState, scriba::jobs::Worker, Arc<Recording>, Arc<Answers>) {
    let paths = paths(dir);
    let (jobs, worker) = Jobs::start(paths.uploads_dir(), pipeline(&paths)).unwrap();
    let (opener, dialog) = (Arc::new(Recording::default()), Arc::new(Answers::default()));
    let state = dialog_state(paths, jobs, opener.clone(), dialog.clone());
    (state, worker, opener, dialog)
}

/// Com o worker: os jobs terminam. O worker vive até o fim do teste.
fn running_state(dir: &tempfile::TempDir) -> (AppState, scriba::jobs::Worker) {
    let (state, worker, _) = recording_state(dir);
    (state, worker)
}

/// Com o worker e o opener que registra.
fn recording_state(dir: &tempfile::TempDir) -> (AppState, scriba::jobs::Worker, Arc<Recording>) {
    let paths = paths(dir);
    let (jobs, worker) = Jobs::start(paths.uploads_dir(), pipeline(&paths)).unwrap();
    let opener = Arc::new(Recording::default());
    (app_state(paths, jobs, opener.clone()), worker, opener)
}

/// O probe do pipeline real (sem modelos: o probe não transcreve).
fn engine_state(dir: &tempfile::TempDir) -> AppState {
    let paths = paths(dir);
    let engine = Engine::with_asr(
        paths.output_dir.clone(),
        Asr::new(Vec::new(), Models::default()),
    );
    let jobs = Jobs::without_worker(paths.uploads_dir(), Arc::new(engine)).unwrap();
    app_state(paths, jobs, Arc::default())
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
    assert!(
        body.contains(&format!(
            "Scriba {} · Transcrição no processador",
            env!("CARGO_PKG_VERSION")
        )),
        "backend no rodapé"
    );
    assert!(!body.contains("Transcrever") && !body.contains("Compactar"));
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
    let (status, _, body) = send(&s, post_jobs(&[file])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("Reunião.MP4") && body.contains("Na fila"),
        "{body}"
    );
    assert!(
        body.contains("2 min · 3 MB · <span class=\"job-card-date\">"),
        "duração do probe, tamanho e data: {body}"
    );
    assert!(body.contains("Cerca de 4 min no processador"), "{body}");
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
        (vec![("arquivo", Some("nota.pdf"), b"x")], formato),
        // O probe recusa depois de gravar: o arquivo é apagado.
        (
            vec![("arquivo", Some("sem-audio.mp4"), b"x")],
            "Este arquivo não tem áudio. Escolha um vídeo com som.",
        ),
        (
            vec![("arquivo", Some(""), b"")],
            "Escolha um arquivo para enviar.",
        ),
        // O segundo arquivo é inválido: o primeiro, já gravado, também é descartado.
        (
            vec![
                ("arquivo", Some("a.wav"), b"x"),
                ("arquivo", Some("b.exe"), b"x"),
            ],
            formato,
        ),
        (
            vec![
                ("arquivo", Some("a.wav"), b"x"),
                ("arquivo", Some("sem-audio.mkv"), b"x"),
            ],
            "Este arquivo não tem áudio.",
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
        // Campos desconhecidos (o "transcrever" de antes) são ignorados.
        ("transcrever", None, b"1".as_slice()),
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
    let mut req = post_jobs(&[("arquivo", Some("a.wav"), b"x")]);
    req.headers_mut().remove(header::ORIGIN);
    assert_eq!(send(&state(&dir), req).await.0, StatusCode::FORBIDDEN);
    assert!(uploads(&dir).is_empty());
}

#[tokio::test]
async fn polling_cancelar_e_remover() {
    let dir = tempfile::tempdir().unwrap();
    let s = state(&dir);
    let file = ("arquivo", Some("a.wav"), b"x".as_slice());
    let (_, _, body) = send(&s, post_jobs(&[file])).await;
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
#[tokio::test]
async fn probe_real_recusa_sem_audio_e_o_que_nao_e_midia() {
    let dir = tempfile::tempdir().unwrap();
    let s = engine_state(&dir);
    let sem_audio = std::fs::read(common::fixture("video-sem-audio.mp4")).unwrap();
    let cases = [
        (
            ("arquivo", Some("tela.mp4"), sem_audio.as_slice()),
            "Este arquivo não tem áudio. Escolha um vídeo com som.",
        ),
        (
            ("arquivo", Some("nota.wav"), b"isto nao e audio".as_slice()),
            "Não reconheci este arquivo como vídeo ou áudio.",
        ),
    ];
    for (part, message) in cases {
        let (status, headers, body) = send(&s, post_jobs(&[part])).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(headers["hx-retarget"], "#erro-envio");
        assert!(body.contains(message), "{body}");
        assert!(uploads(&dir).is_empty(), "{message}");
    }

    // Um M4A de verdade passa, com a duração do probe e sem estimativa (nenhum modelo).
    let audio = std::fs::read(common::fixture("fala.m4a")).unwrap();
    let (status, _, body) = send(&s, post_jobs(&[("arquivo", Some("fala.m4a"), &audio)])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("11 s · "), "{body}");
    assert!(!body.contains("Cerca de"), "{body}");
}

/// Espera o card do job chegar ao fim do polling (286) e devolve o HTML dele.
async fn final_card(s: &AppState, id: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (status, _, card) = send(
            s,
            get(&format!("/api/jobs/{id}")).body(Body::empty()).unwrap(),
        )
        .await;
        if status.as_u16() == 286 {
            return card;
        }
        assert!(Instant::now() < deadline, "o job não terminou: {card}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn tentar_de_novo_volta_para_a_fila() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker) = running_state(&dir);
    let (_, _, body) = send(&s, post_jobs(&[("arquivo", Some("falha.wav"), b"x")])).await;
    let id = card_ids(&body).remove(0);
    let card = final_card(&s, &id).await;
    assert!(
        card.contains("Falhou de propósito.") && card.contains("Tentar de novo"),
        "{card}"
    );
    assert_eq!(uploads(&dir).len(), 1, "o envio fica para a nova tentativa");

    let retry = format!("/api/jobs/{id}/retry");
    let (status, _, card) = send(&s, with_origin(Request::post(&retry))).await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert!(
        card.contains(r#"hx-trigger="every 1s""#),
        "volta a fazer polling: {card}"
    );
    assert!(card.contains("falha.wav na fila."), "{card}");

    // Falha de novo (o pipeline sempre falha com esse nome); um job concluído não volta.
    final_card(&s, &id).await;
    let (_, _, body) = send(&s, post_jobs(&[("arquivo", Some("ok.wav"), b"x")])).await;
    let ok = card_ids(&body).remove(0);
    final_card(&s, &ok).await;
    let (status, _, _) = send(
        &s,
        with_origin(Request::post(format!("/api/jobs/{ok}/retry"))),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    for uri in [
        "/api/jobs/01JAZ5Y3V8K9M2N4P6Q7R8S9T0/retry",
        "/api/jobs/x/retry",
    ] {
        assert_eq!(
            send(&s, with_origin(Request::post(uri))).await.0,
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn remover_concluido_exige_confirmacao() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker) = running_state(&dir);
    let (_, _, body) = send(&s, post_jobs(&[("arquivo", Some("reuniao.mp4"), b"x")])).await;
    let id = card_ids(&body).remove(0);
    let card = final_card(&s, &id).await;
    assert!(
        card.contains(&format!(r#"hx-delete="/api/jobs/{id}?confirm=1""#))
            && card.contains("A pasta “pasta reuniao.mp4” vai para a Lixeira."),
        "{card}"
    );
    let folder = dir.path().join("saidas").join("pasta reuniao.mp4");
    assert!(folder.is_dir());

    let job = format!("/api/jobs/{id}");
    let (status, _, _) = send(&s, with_origin(Request::delete(&job))).await;
    assert_eq!(status, StatusCode::CONFLICT, "sem confirmação");
    assert!(folder.is_dir());
    let (status, _, body) =
        send(&s, with_origin(Request::delete(format!("{job}?confirm=1")))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!folder.exists(), "a pasta foi para a Lixeira");
    assert_eq!(
        send(&s, get(&job).body(Body::empty()).unwrap()).await.0,
        StatusCode::NOT_FOUND
    );
}
// Resultado: leitor, downloads e "Abrir pasta" ----------------------------------------

/// Envia um arquivo, espera concluir e devolve o id e a pasta da saída.
async fn done_job(s: &AppState, dir: &tempfile::TempDir, name: &str) -> (String, PathBuf) {
    let (_, _, body) = send(s, post_jobs(&[("arquivo", Some(name), b"x")])).await;
    let id = card_ids(&body).remove(0);
    let card = final_card(s, &id).await;
    assert!(card.contains("Concluído"), "{card}");
    (id, dir.path().join("saidas").join(format!("pasta {name}")))
}

async fn get_page(s: &AppState, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    send(s, get(uri).body(Body::empty()).unwrap()).await
}

#[tokio::test]
async fn card_concluido_tem_as_acoes_do_resultado() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker) = running_state(&dir);
    let (_, _, body) = send(&s, post_jobs(&[("arquivo", Some("Reunião.mp4"), b"x")])).await;
    let id = card_ids(&body).remove(0);
    let card = final_card(&s, &id).await;
    for part in [
        format!(r#"href="/jobs/{id}""#),
        format!(r#"href="/api/jobs/{id}/transcricao.txt" download="Reunião - transcrição.txt""#),
        format!(r#"href="/api/jobs/{id}/transcricao.md" download="Reunião - transcrição.md""#),
        format!(r#"hx-post="/api/jobs/{id}/open""#),
    ] {
        assert!(card.contains(&part), "{part}\n{card}");
    }
}

#[tokio::test]
async fn leitor_mostra_a_transcricao_do_disco() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker) = running_state(&dir);
    let (id, folder) = done_job(&s, &dir, "reuniao.mp4").await;

    let (status, headers, page) = get_page(&s, &format!("/jobs/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(
        page.contains("<title>reuniao.mp4 · Scriba</title>"),
        "{page}"
    );
    assert!(
        page.contains(r#"<h1 id="transcricao-titulo" class="transcript-title">reuniao.mp4</h1>"#)
    );
    assert!(
        page.contains(r#"<span class="transcript-timestamp">00:00:09</span> Segundo parágrafo."#),
        "{page}"
    );
    assert!(page.contains("reunião."), "{page}");
    assert!(!page.contains("<boa>"), "o texto é escapado: {page}");
    // O `.txt` inteiro vai no <template> do Copiar.
    let txt = std::fs::read_to_string(folder.join(transcript::TXT_FILE)).unwrap();
    assert!(txt.starts_with("Arquivo: reuniao.mp4\r\n"), "{txt}");
    assert!(
        page.contains(r#"<template id="texto-transcricao">Arquivo: reuniao.mp4"#),
        "{page}"
    );

    // Sem fala: o estado vazio, com as ações.
    let (empty, _) = done_job(&s, &dir, "sem-fala.wav").await;
    let (status, _, page) = get_page(&s, &format!("/jobs/{empty}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("Nenhuma fala encontrada neste arquivo."),
        "{page}"
    );
    assert!(page.contains("Copiar texto"), "{page}");

    // `.txt` apagado fora do app: o alerta, sem as ações.
    std::fs::remove_file(folder.join(transcript::TXT_FILE)).unwrap();
    let (status, _, page) = get_page(&s, &format!("/jobs/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Não encontrei a transcrição em "), "{page}");
    assert!(page.contains("pasta reuniao.mp4."), "{page}");
    assert!(!page.contains("Copiar texto"), "{page}");

    // Fora da lista, ou ainda não concluído: 404 com a página.
    let (_, _, body) = send(&s, post_jobs(&[("arquivo", Some("falha.wav"), b"x")])).await;
    let failed = card_ids(&body).remove(0);
    final_card(&s, &failed).await;
    for uri in [
        "/jobs/01JAZ5Y3V8K9M2N4P6Q7R8S9T0".to_string(),
        "/jobs/x".to_string(),
        format!("/jobs/{failed}"),
    ] {
        let (status, _, page) = get_page(&s, &uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(
            page.contains("Não encontrei esta transcrição na lista de Recentes."),
            "{page}"
        );
    }

    // Sem o cookie, como toda rota.
    let res = router(s.clone())
        .oneshot(
            Request::get(format!("/jobs/{id}"))
                .header(header::HOST, HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn downloads_do_txt_e_do_md() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker) = running_state(&dir);
    let (id, folder) = done_job(&s, &dir, "Reunião.mp4").await;

    let cases = [
        ("txt", transcript::TXT_FILE, "text/plain; charset=utf-8"),
        ("md", transcript::MD_FILE, "text/markdown; charset=utf-8"),
    ];
    for (ext, file, mime) in cases {
        let uri = format!("/api/jobs/{id}/transcricao.{ext}");
        let (status, headers, body) = get_page(&s, &uri).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(headers[header::CONTENT_TYPE], mime);
        assert_eq!(
            headers[header::CONTENT_DISPOSITION],
            format!(
                "attachment; filename=\"transcricao.{ext}\"; \
                 filename*=UTF-8''Reuni%C3%A3o%20-%20transcri%C3%A7%C3%A3o.{ext}"
            )
        );
        assert_eq!(body, std::fs::read_to_string(folder.join(file)).unwrap());
    }
    let (_, _, md) = get_page(&s, &format!("/api/jobs/{id}/transcricao.md")).await;
    assert!(md.starts_with("# Reunião.mp4\n"), "{md}");
    assert!(md.contains("**[00:00:09]** Segundo parágrafo."), "{md}");

    // Arquivo apagado fora do app: 404 com a mensagem do toast.
    std::fs::remove_file(folder.join(transcript::MD_FILE)).unwrap();
    let (status, _, body) = get_page(&s, &format!("/api/jobs/{id}/transcricao.md")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        "Não encontrei a transcrição na pasta “pasta Reunião.mp4”. Ela foi movida ou apagada."
    );
    for uri in [
        "/api/jobs/01JAZ5Y3V8K9M2N4P6Q7R8S9T0/transcricao.txt".to_string(),
        "/api/jobs/x/transcricao.md".to_string(),
        format!("/api/jobs/{id}/transcricao.pdf"),
    ] {
        assert_eq!(get_page(&s, &uri).await.0, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn abrir_pasta_chama_o_opener_com_a_pasta_do_job() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker, opener) = recording_state(&dir);
    let (id, folder) = done_job(&s, &dir, "reuniao.mp4").await;
    let open = format!("/api/jobs/{id}/open");

    let (status, _, body) = send(&s, with_origin(Request::post(&open))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(
        *opener.opened.lock().unwrap(),
        std::slice::from_ref(&folder)
    );

    // Sem Origin, como todo POST.
    let res = router(s.clone())
        .oneshot(
            Request::post(&open)
                .header(header::HOST, HOST)
                .header(header::COOKIE, format!("scriba_token={TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    opener.fail.store(true, Ordering::Relaxed);
    let (status, _, body) = send(&s, with_origin(Request::post(&open))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, "Não consegui abrir a pasta.");
    opener.fail.store(false, Ordering::Relaxed);

    // Pasta apagada fora do app: a mensagem, sem chamar o opener.
    std::fs::remove_dir_all(&folder).unwrap();
    let (status, _, body) = send(&s, with_origin(Request::post(&open))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        "Não encontrei a pasta “pasta reuniao.mp4”. Ela foi movida ou apagada."
    );
    assert_eq!(opener.opened.lock().unwrap().len(), 1);

    for uri in [
        "/api/jobs/01JAZ5Y3V8K9M2N4P6Q7R8S9T0/open",
        "/api/jobs/x/open",
    ] {
        let (status, _, _) = send(&s, with_origin(Request::post(uri))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
}
// "Guardar o áudio?" -------------------------------------------------------------------

/// Faz o polling do áudio (`?audio=1`) até o 286 e devolve os cabeçalhos e o card.
async fn final_audio_card(s: &AppState, id: &str) -> (axum::http::HeaderMap, String) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let uri = format!("/api/jobs/{id}?audio=1");
        let (status, headers, card) = get_page(s, &uri).await;
        if status.as_u16() == 286 {
            return (headers, card);
        }
        assert_eq!(status, StatusCode::OK, "{card}");
        assert!(Instant::now() < deadline, "o áudio não terminou: {card}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn post(s: &AppState, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    send(s, with_origin(Request::post(uri))).await
}

#[tokio::test]
async fn card_concluido_pergunta_e_descartar_apaga_o_temporario() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker) = running_state(&dir);
    let (id, _) = done_job(&s, &dir, "reuniao.mp4").await;
    let card = final_card(&s, &id).await;
    for part in [
        "Guardar o áudio?".to_string(),
        format!(r#"hx-post="/api/jobs/{id}/audio/save""#),
        format!(r#"hx-post="/api/jobs/{id}/audio/discard""#),
    ] {
        assert!(card.contains(&part), "{part}\n{card}");
    }
    assert_eq!(uploads(&dir).len(), 1, "o temporário espera a resposta");

    let discard = format!("/api/jobs/{id}/audio/discard");
    let (status, _, card) = post(&s, &discard).await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert!(!card.contains("Guardar o áudio?"), "{card}");
    assert!(card.contains("Áudio de reuniao.mp4 descartado."), "{card}");
    assert!(uploads(&dir).is_empty());

    let (status, _, body) = post(&s, &discard).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body, "O áudio deste job já foi guardado ou descartado.");
    let (status, _, _) = post(&s, &format!("/api/jobs/{id}/audio/save")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    for uri in [
        "/api/jobs/01JAZ5Y3V8K9M2N4P6Q7R8S9T0/audio/discard",
        "/api/jobs/x/audio/save",
        "/api/jobs/x/audio/cancel",
        "/api/jobs/x/audio/show",
    ] {
        assert_eq!(post(&s, uri).await.0, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn job_que_nao_concluiu_nao_responde_a_pergunta() {
    let dir = tempfile::tempdir().unwrap();
    // Sem worker: o job fica na fila.
    let s = state(&dir);
    let (_, _, body) = send(&s, post_jobs(&[("arquivo", Some("a.mp4"), b"x")])).await;
    let id = card_ids(&body).remove(0);
    for action in ["discard", "save"] {
        let (status, _, body) = post(&s, &format!("/api/jobs/{id}/audio/{action}")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{action}");
        assert_eq!(body, "Espere a transcrição terminar.");
    }
    let (status, _, card) = post(&s, &format!("/api/jobs/{id}/audio/cancel")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "cancelar fora da gravação não muda nada"
    );
    assert!(card.contains("Na fila"), "{card}");
    assert_eq!(uploads(&dir).len(), 1);
}

#[tokio::test]
async fn guardar_grava_onde_a_janela_mandar_e_lembra_a_pasta() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker, opener, dialog) = audio_state(&dir);
    let (id, folder) = done_job(&s, &dir, "reuniao.mp4").await;
    let audios = dir.path().join("Áudios");
    std::fs::create_dir(&audios).unwrap();
    let chosen = audios.join("Reunião.m4a");
    *dialog.answer.lock().unwrap() = Some(chosen.clone());

    let (status, _, card) = post(&s, &format!("/api/jobs/{id}/audio/save")).await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert!(
        card.contains(&format!(r#"hx-get="/api/jobs/{id}?audio=1""#)),
        "acompanha pelo polling: {card}"
    );
    let (_, card) = final_audio_card(&s, &id).await;
    let saved = format!(
        r#"Áudio guardado em <span class="audio-prompt-path">{}</span>"#,
        chosen.display()
    );
    assert!(card.contains(&saved), "{card}");
    assert!(card.contains("Áudio de reuniao.mp4 guardado."), "{card}");
    assert!(!card.contains("Guardar o áudio?"), "{card}");
    assert_eq!(
        std::fs::read_to_string(&chosen).unwrap(),
        "áudio de reuniao.mp4"
    );
    assert!(uploads(&dir).is_empty(), "guardado apaga o temporário");
    // Na 1ª vez, a janela abre na pasta da transcrição, com o nome dela.
    assert_eq!(
        dialog.asked.lock().unwrap().clone(),
        [SaveRequest {
            file_name: "pasta reuniao.mp4.m4a".into(),
            dir: folder.clone(),
        }]
    );
    assert_eq!(
        Config::load(&dir.path().join("config.json")).audio_dir,
        Some(audios.clone())
    );

    // "Mostrar no Explorer": o arquivo selecionado na pasta dele.
    let show = format!("/api/jobs/{id}/audio/show");
    assert_eq!(post(&s, &show).await.0, StatusCode::NO_CONTENT);
    assert_eq!(
        *opener.revealed.lock().unwrap(),
        std::slice::from_ref(&chosen)
    );
    std::fs::remove_file(&chosen).unwrap();
    let (status, _, body) = post(&s, &show).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        "Não encontrei o áudio “Reunião.m4a”. Ele foi movido ou apagado."
    );

    // A próxima abre na pasta lembrada.
    let (other, other_folder) = done_job(&s, &dir, "aula.mp3").await;
    let (status, _, body) = post(&s, &format!("/api/jobs/{other}/audio/show")).await;
    assert_eq!(status, StatusCode::CONFLICT, "nada guardado ainda");
    assert_eq!(body, "O áudio deste job não foi guardado.");
    *dialog.answer.lock().unwrap() = Some(folder.join("na pasta da outra.m4a"));
    post(&s, &format!("/api/jobs/{other}/audio/save")).await;
    final_audio_card(&s, &other).await;
    assert_eq!(dialog.asked.lock().unwrap()[1].dir, audios);

    // A última foi a pasta de outra transcrição: a janela abre na desta.
    let (third, third_folder) = done_job(&s, &dir, "palestra.wav").await;
    *dialog.answer.lock().unwrap() = None;
    post(&s, &format!("/api/jobs/{third}/audio/save")).await;
    final_audio_card(&s, &third).await;
    assert_eq!(dialog.asked.lock().unwrap()[2].dir, third_folder);
    assert_ne!(other_folder, third_folder);
}

#[tokio::test]
async fn janela_cancelada_nao_apaga_nada_e_mantem_a_pergunta() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker, _, dialog) = audio_state(&dir);
    let (id, _) = done_job(&s, &dir, "reuniao.mp4").await;
    let (status, _, _) = post(&s, &format!("/api/jobs/{id}/audio/save")).await;
    assert_eq!(status, StatusCode::OK);
    let (headers, card) = final_audio_card(&s, &id).await;
    assert!(card.contains("Guardar o áudio?"), "{card}");
    assert!(!headers.contains_key("HX-Trigger"));
    assert_eq!(uploads(&dir).len(), 1);
    assert_eq!(dialog.asked.lock().unwrap().len(), 1);
    assert_eq!(
        Config::load(&dir.path().join("config.json")).audio_dir,
        None,
        "nada escolhido, nada lembrado"
    );
}

#[tokio::test]
async fn erro_ao_guardar_vira_toast_e_a_pergunta_volta() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker, _, dialog) = audio_state(&dir);
    let (id, _) = done_job(&s, &dir, "audio-ruim.mp4").await;
    let chosen = dir.path().join("ruim.m4a");
    *dialog.answer.lock().unwrap() = Some(chosen.clone());
    post(&s, &format!("/api/jobs/{id}/audio/save")).await;
    let (headers, card) = final_audio_card(&s, &id).await;
    assert!(card.contains("Guardar o áudio?"), "{card}");
    let trigger = headers["HX-Trigger"].to_str().unwrap();
    assert!(trigger.is_ascii(), "{trigger}");
    let event: serde_json::Value = serde_json::from_str(trigger).unwrap();
    assert_eq!(event["scriba-erro"], "Não consegui guardar o áudio.");
    assert!(!chosen.exists());
    assert_eq!(uploads(&dir).len(), 1);
    // A lista não repete o toast.
    let (_, headers, _) = get_page(&s, "/api/jobs").await;
    assert!(!headers.contains_key("HX-Trigger"));
}

#[tokio::test]
async fn cancelar_a_gravacao_e_remover_so_depois() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _worker, _, dialog) = audio_state(&dir);
    let (id, _) = done_job(&s, &dir, "audio-sem-fim.mp4").await;
    let chosen = dir.path().join("sem-fim.m4a");
    *dialog.answer.lock().unwrap() = Some(chosen.clone());
    post(&s, &format!("/api/jobs/{id}/audio/save")).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, _, card) = get_page(&s, &format!("/api/jobs/{id}?audio=1")).await;
        if card.contains("Guardando áudio… 10%") {
            assert!(
                card.contains(&format!(r#"hx-post="/api/jobs/{id}/audio/cancel""#)),
                "{card}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "{card}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let remove = format!("/api/jobs/{id}?confirm=1");
    let (status, _, body) = send(&s, with_origin(Request::delete(&remove))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body, "Cancele o áudio antes de remover.");

    let (status, _, card) = post(&s, &format!("/api/jobs/{id}/audio/cancel")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(card.contains(r#"aria-disabled="true""#), "{card}");
    let (_, card) = final_audio_card(&s, &id).await;
    assert!(card.contains("Guardar o áudio?"), "{card}");
    assert!(!chosen.exists() && !scriba::jobs::part_path(&chosen).exists());
    assert_eq!(uploads(&dir).len(), 1);
    let (status, _, _) = send(&s, with_origin(Request::delete(&remove))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(uploads(&dir).is_empty(), "remover apaga o temporário");
}
