//! O resultado de um job concluído: o leitor da transcrição (`/jobs/{id}`, lido do `.txt` em
//! disco), os downloads do `.txt` e do `.md` e o "Abrir pasta".

use std::io;
use std::path::Path as FsPath;

use askama::Template;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::AppState;
use super::jobs::Card;
use super::pages::{Layout, render};
use crate::config::Theme;
use crate::jobs::{Job, Outputs, State as JobState};
use crate::transcript::{self, clean_name};

/// O que o leitor mostra. Sem `loaded`, só o título, os metadados e o alerta.
pub struct ReaderView {
    pub title: String,
    /// O job, para os metadados; `None` quando ele não está na lista.
    pub job: Option<Card>,
    pub alert: Option<String>,
    pub loaded: Option<Loaded>,
}

/// A transcrição lida do disco.
pub struct Loaded {
    pub id: String,
    pub txt_name: String,
    pub md_name: String,
    /// O `.txt` inteiro, para o "Copiar texto".
    pub text: String,
    /// Vazio: o arquivo não tem fala.
    pub paragraphs: Vec<ParagraphView>,
}

pub struct ParagraphView {
    /// `00:00:08`; `None` para texto fora do formato.
    pub time: Option<String>,
    pub text: String,
}

#[derive(Template)]
#[template(path = "transcript.html")]
struct ReaderPage {
    layout: Layout,
    theme: Theme,
    view: ReaderView,
}

/// Qual dos dois arquivos da transcrição.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Txt,
    Md,
}

impl Kind {
    fn ext(self) -> &'static str {
        match self {
            Self::Txt => "txt",
            Self::Md => "md",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Txt => "text/plain; charset=utf-8",
            Self::Md => "text/markdown; charset=utf-8",
        }
    }

    fn path(self, outputs: &Outputs) -> &FsPath {
        match self {
            Self::Txt => &outputs.txt,
            Self::Md => &outputs.md,
        }
    }
}

/// "Reunião - transcrição.md": o nome do arquivo original, sem a extensão e limpo para o
/// Windows.
pub fn download_name(job_name: &str, kind: Kind) -> String {
    let stem = FsPath::new(job_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(job_name);
    format!("{} - transcrição.{}", clean_name(stem), kind.ext())
}

/// `attachment` com o nome em UTF-8 (`filename*`, RFC 5987) e um nome ASCII fixo para quem
/// não entende o `filename*`.
pub fn content_disposition(job_name: &str, kind: Kind) -> String {
    format!(
        "attachment; filename=\"transcricao.{}\"; filename*=UTF-8''{}",
        kind.ext(),
        encode_rfc5987(&download_name(job_name, kind))
    )
}

/// Percent-encoding do `ext-value`: só os `attr-char` passam como estão.
fn encode_rfc5987(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// O nome da pasta da saída, como o card e as mensagens mostram.
pub fn folder_label(dir: &FsPath) -> String {
    dir.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// O job, se ele existe e está concluído.
fn done_job(state: &AppState, id: &str) -> Option<(Job, Outputs)> {
    let job = state.jobs.get(id.parse().ok()?)?;
    match &job.state {
        JobState::Done { outputs, .. } => {
            let outputs = outputs.clone();
            Some((job, outputs))
        }
        _ => None,
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Job não encontrado.").into_response()
}

/// Monta o leitor de um job concluído a partir do `.txt` em disco.
pub fn view(job: &Job, outputs: &Outputs, read: io::Result<Vec<u8>>) -> ReaderView {
    let card = Card::new(job);
    let (alert, loaded) = match read {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let paragraphs = transcript::parse(&text)
                .blocks
                .into_iter()
                .map(|b| ParagraphView {
                    time: b.start_ms.map(transcript::timestamp),
                    text: b.text,
                })
                .collect();
            let loaded = Loaded {
                id: card.id.clone(),
                txt_name: download_name(&job.name, Kind::Txt),
                md_name: download_name(&job.name, Kind::Md),
                text,
                paragraphs,
            };
            (None, Some(loaded))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (
            Some(format!(
                "Não encontrei a transcrição em {}.",
                outputs.dir.display()
            )),
            None,
        ),
        Err(e) => {
            eprintln!("não consegui ler {}: {e}", outputs.txt.display());
            (
                Some(format!(
                    "Não consegui ler a transcrição em {}.",
                    outputs.dir.display()
                )),
                None,
            )
        }
    };
    ReaderView {
        title: job.name.clone(),
        job: Some(card),
        alert,
        loaded,
    }
}

/// O leitor de um id que não está na lista (a lista começa vazia a cada execução).
pub fn unknown_view() -> ReaderView {
    ReaderView {
        title: "Transcrição".into(),
        job: None,
        alert: Some("Não encontrei esta transcrição na lista de Recentes.".into()),
        loaded: None,
    }
}

/// `GET /jobs/{id}`.
pub async fn page(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let (status, view) = match done_job(&state, &id) {
        Some((job, outputs)) => {
            let read = tokio::fs::read(&outputs.txt).await;
            (StatusCode::OK, view(&job, &outputs, read))
        }
        None => (StatusCode::NOT_FOUND, unknown_view()),
    };
    let mut response = render(&ReaderPage {
        layout: Layout::new(state.jobs.backend()),
        theme: state.config().theme,
        view,
    });
    if response.status().is_success() {
        *response.status_mut() = status;
    }
    response
}

pub async fn download_txt(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    download(&state, &id, Kind::Txt).await
}

pub async fn download_md(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    download(&state, &id, Kind::Md).await
}

/// O arquivo como anexo. Os erros vêm em texto, que a tela mostra num toast.
async fn download(state: &AppState, id: &str, kind: Kind) -> Response {
    let Some((job, outputs)) = done_job(state, id) else {
        return not_found();
    };
    let path = kind.path(&outputs);
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let disposition = HeaderValue::from_str(&content_disposition(&job.name, kind))
                .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
            (
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(kind.mime())),
                    (header::CONTENT_DISPOSITION, disposition),
                ],
                bytes,
            )
                .into_response()
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (
            StatusCode::NOT_FOUND,
            format!(
                "Não encontrei a transcrição na pasta “{}”. Ela foi movida ou apagada.",
                folder_label(&outputs.dir)
            ),
        )
            .into_response(),
        Err(e) => {
            eprintln!("não consegui ler {}: {e}", path.display());
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Não consegui ler a transcrição. Tente de novo.",
            )
                .into_response()
        }
    }
}

/// `POST /api/jobs/{id}/open`: a pasta da saída no Explorer. 204 quando abre.
pub async fn open_folder(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some((_, outputs)) = done_job(&state, &id) else {
        return not_found();
    };
    if !outputs.dir.is_dir() {
        return (
            StatusCode::NOT_FOUND,
            format!(
                "Não encontrei a pasta “{}”. Ela foi movida ou apagada.",
                folder_label(&outputs.dir)
            ),
        )
            .into_response();
    }
    let opener = state.opener.clone();
    let dir = outputs.dir.clone();
    match tokio::task::spawn_blocking(move || opener.open(&dir)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        result => {
            if let Ok(Err(e)) = result {
                eprintln!("não consegui abrir {}: {e}", outputs.dir.display());
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Não consegui abrir a pasta.",
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::LocalTime;
    use crate::jobs::{Estimate, Media};

    #[test]
    fn nome_do_download() {
        assert_eq!(
            download_name("Reunião.mp4", Kind::Md),
            "Reunião - transcrição.md"
        );
        assert_eq!(
            download_name("a\"b;c.d.wav", Kind::Txt),
            "a b;c.d - transcrição.txt"
        );
        assert_eq!(
            download_name("CON.mp3", Kind::Txt),
            "_CON - transcrição.txt"
        );
        assert_eq!(
            download_name("?.mp3", Kind::Txt),
            "Sem nome - transcrição.txt"
        );
    }

    #[test]
    fn content_disposition_em_utf8() {
        assert_eq!(
            content_disposition("Reunião.mp4", Kind::Md),
            "attachment; filename=\"transcricao.md\"; \
             filename*=UTF-8''Reuni%C3%A3o%20-%20transcri%C3%A7%C3%A3o.md"
        );
        let value = content_disposition("a\"b;c (1).wav", Kind::Txt);
        assert!(
            value.ends_with("filename*=UTF-8''a%20b%3Bc%20%281%29%20-%20transcri%C3%A7%C3%A3o.txt"),
            "{value}"
        );
        assert!(HeaderValue::from_str(&value).is_ok());
    }

    fn job(outputs: Outputs) -> Job {
        Job {
            id: "01JAZ5Y3V8K9M2N4P6Q7R8S9T0".parse().unwrap(),
            name: "Reunião <equipe>.mp4".into(),
            media: Media::Video,
            size: 312 * 1024 * 1024,
            duration_ms: 47 * 60_000 + 12_000,
            created: LocalTime {
                year: 2026,
                month: 10,
                day: 20,
                hour: 14,
                minute: 30,
                second: 5,
                offset_minutes: -180,
            },
            estimate: Some(Estimate {
                secs: 74,
                gpu: true,
            }),
            input: "nada".into(),
            state: JobState::Done {
                outputs,
                audio: crate::jobs::Audio::PENDING,
            },
        }
    }

    fn outputs() -> Outputs {
        let dir = std::path::PathBuf::from(r"C:\Saidas\2026-10-20 1430 - Reunião equipe");
        Outputs {
            txt: dir.join(transcript::TXT_FILE),
            md: dir.join(transcript::MD_FILE),
            dir,
        }
    }

    #[derive(Template)]
    #[template(path = "partials/transcript_view.html")]
    struct ViewTemplate<'a> {
        view: &'a ReaderView,
    }

    fn html(view: &ReaderView) -> String {
        ViewTemplate { view }.render().unwrap()
    }

    #[test]
    fn leitor_em_cada_caso() {
        let o = outputs();
        let txt = "Arquivo: Reunião <equipe>.mp4\r\nData: 20/10/2026 14:30\r\nDuração: 00:47:12\r\n\
                   \r\n[00:00:00] Bom dia & <b>até</b> logo.\r\n\
                   \r\n[00:00:08] Segundo parágrafo.\r\n\
                   \r\nNota editada à mão\r\nem duas linhas.\r\n";
        let cases = [
            ("leitor", view(&job(o.clone()), &o, Ok(txt.into()))),
            (
                "leitor_sem_fala",
                view(
                    &job(o.clone()),
                    &o,
                    Ok(b"Arquivo: x.mp4\r\nData: 20/10/2026 14:30\r\nDura\xc3\xa7\xc3\xa3o: 00:00:05\r\n".to_vec()),
                ),
            ),
            (
                "leitor_ausente",
                view(&job(o.clone()), &o, Err(io::ErrorKind::NotFound.into())),
            ),
            ("leitor_desconhecido", unknown_view()),
        ];
        for (name, view) in cases {
            insta::assert_snapshot!(name, html(&view));
        }
    }
}
