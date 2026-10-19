//! Envio de arquivos em streaming para o disco e as rotas da fila, que respondem fragmentos
//! HTML para o htmx: o card de cada job e, no fim dele, um aviso para leitores de tela.

use std::io;
use std::path::{Path as FsPath, PathBuf};

use askama::Template;
use axum::extract::multipart::Field;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use tokio::io::{AsyncWriteExt, BufWriter};

use super::AppState;
use crate::jobs::{FORMATS, Job, JobId, Jobs, Media, Options, RemoveError, State as JobState};

/// Resposta que encerra o polling do htmx.
const STOP_POLLING: u16 = 286;

pub const NO_OPTIONS: &str = "Escolha Transcrever, Compactar ou os dois.";

/// "Formato não suportado. Use MP4, MOV, MKV, MP3, M4A, WAV ou OGG."
pub fn format_error() -> String {
    let names: Vec<String> = FORMATS.iter().map(|(e, _)| e.to_uppercase()).collect();
    match names.split_last() {
        Some((last, rest)) => format!("Formato não suportado. Use {} ou {last}.", rest.join(", ")),
        None => "Formato não suportado.".into(),
    }
}

/// Valor do `accept` do input de arquivo: ".mp4,.mov,…".
pub fn accept() -> String {
    FORMATS
        .iter()
        .map(|(e, _)| format!(".{e}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Dados de um card, já formatados para o template.
pub struct Card {
    pub id: String,
    pub name: String,
    pub video: bool,
    pub size: String,
    pub badge: &'static str,
    /// Sufixo da variante do `status-badge` (`active`, `success`, `error`); vazio no neutro.
    pub badge_variant: &'static str,
    /// Na fila ou rodando: o card faz polling e oferece Cancelar.
    pub active: bool,
    pub canceling: bool,
    /// Etapa com a porcentagem, só enquanto roda.
    pub stage: Option<String>,
    pub pct: u8,
    pub error: Option<String>,
}

impl Card {
    pub fn new(job: &Job) -> Self {
        let (badge, badge_variant) = match &job.state {
            JobState::Queued => ("Na fila", ""),
            JobState::Running { .. } => ("Processando", "active"),
            JobState::Done { .. } => ("Concluído", "success"),
            JobState::Failed { .. } => ("Falhou", "error"),
            JobState::Canceled => ("Cancelado", ""),
        };
        let (stage, pct, canceling) = match &job.state {
            JobState::Running {
                canceling: true,
                pct,
                ..
            } => (Some("Cancelando…".to_string()), *pct, true),
            JobState::Running { stage, pct, .. } => {
                (Some(format!("{}… {pct}%", stage.label())), *pct, false)
            }
            _ => (None, 0, false),
        };
        Self {
            id: job.id.to_string(),
            name: job.name.clone(),
            video: job.media == Media::Video,
            size: format_size(job.size),
            badge,
            badge_variant,
            active: !job.state.is_terminal(),
            canceling,
            stage,
            pct,
            error: match &job.state {
                JobState::Failed { message } => Some(message.clone()),
                _ => None,
            },
        }
    }
}

/// Cards e, opcionalmente, o aviso fora de banda para a região `#avisos`.
#[derive(Template)]
#[template(path = "partials/jobs.html")]
struct Fragment {
    jobs: Vec<Card>,
    notice: Option<String>,
}

#[derive(Template)]
#[template(path = "partials/upload_error.html")]
struct UploadErrorTemplate {
    message: String,
}

fn fragment(status: StatusCode, jobs: Vec<Card>, notice: Option<String>) -> Response {
    match (Fragment { jobs, notice }).render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Aviso de fim de job para leitores de tela.
fn ended(job: &Job) -> String {
    let how = match &job.state {
        JobState::Done { .. } => "concluído",
        JobState::Failed { .. } => "falhou",
        JobState::Canceled => "cancelado",
        _ => "em andamento",
    };
    format!("{}: {how}.", job.name)
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Job não encontrado.").into_response()
}

pub async fn list(State(state): State<AppState>) -> Response {
    let cards = state.jobs.list().iter().map(Card::new).collect();
    fragment(StatusCode::OK, cards, None)
}

/// Polling do card: enquanto o job está ativo, 200; no fim, 286 com o card final.
pub async fn show(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(job) = id.parse().ok().and_then(|id| state.jobs.get(id)) else {
        return not_found();
    };
    if job.state.is_terminal() {
        let status = StatusCode::from_u16(STOP_POLLING).unwrap_or(StatusCode::OK);
        fragment(status, vec![Card::new(&job)], Some(ended(&job)))
    } else {
        fragment(StatusCode::OK, vec![Card::new(&job)], None)
    }
}

pub async fn cancel(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<JobId>() else {
        return not_found();
    };
    let was_active = state.jobs.get(id).is_some_and(|j| !j.state.is_terminal());
    let Some(job) = state.jobs.cancel(id) else {
        return not_found();
    };
    let notice = (was_active && job.state.is_terminal()).then(|| ended(&job));
    fragment(StatusCode::OK, vec![Card::new(&job)], notice)
}

/// Corpo vazio: o htmx troca o card por nada.
pub async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<JobId>() else {
        return not_found();
    };
    match state.jobs.remove(id) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(RemoveError::NotFound) => not_found(),
        Err(RemoveError::Active) => {
            (StatusCode::CONFLICT, "Cancele o job antes de remover.").into_response()
        }
    }
}

/// `POST /api/jobs` (multipart): opções `transcrever`/`compactar` e um ou mais campos
/// `arquivo`, gravados em streaming em `uploads/`. Cada arquivo vira um job.
pub async fn create(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let mut received = Vec::new();
    match receive(&state.jobs, &mut multipart, &mut received).await {
        Ok(()) => {
            let notice = match received.as_slice() {
                [one] => format!("{} na fila.", one.name),
                many => format!("{} arquivos na fila.", many.len()),
            };
            let cards: Vec<Card> = received.iter().rev().map(Card::new).collect();
            for job in received {
                state.jobs.submit(job);
            }
            fragment(StatusCode::OK, cards, Some(notice))
        }
        Err(e) => {
            for job in &received {
                let _ = std::fs::remove_file(&job.input);
            }
            // Lê o resto do corpo: assim o navegador recebe a resposta em vez de ver a
            // conexão cair no meio do envio.
            while let Ok(Some(mut field)) = multipart.next_field().await {
                while let Ok(Some(_)) = field.chunk().await {}
            }
            upload_error(&e)
        }
    }
}

#[derive(Debug)]
enum UploadError {
    Format,
    NoOptions,
    NoFile,
    /// O cliente caiu ou mandou um multipart inválido.
    Interrupted,
    Io(io::Error),
}

impl From<io::Error> for UploadError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

fn upload_error(e: &UploadError) -> Response {
    let (status, message) = match e {
        UploadError::Format => (StatusCode::UNPROCESSABLE_ENTITY, format_error()),
        UploadError::NoOptions => (StatusCode::UNPROCESSABLE_ENTITY, NO_OPTIONS.into()),
        UploadError::NoFile => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Escolha um arquivo para enviar.".into(),
        ),
        UploadError::Interrupted => (
            StatusCode::BAD_REQUEST,
            "O envio foi interrompido. Tente de novo.".into(),
        ),
        UploadError::Io(err) => {
            eprintln!("erro gravando o envio: {err}");
            (
                StatusCode::INSUFFICIENT_STORAGE,
                "Não consegui salvar o arquivo. Confira o espaço livre em disco e tente de novo."
                    .into(),
            )
        }
    };
    let mut response = match (UploadErrorTemplate { message }).render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    };
    let headers = response.headers_mut();
    headers.insert("HX-Retarget", HeaderValue::from_static("#erro-envio"));
    headers.insert("HX-Reswap", HeaderValue::from_static("innerHTML"));
    response
}

/// Lê os campos em ordem e grava cada arquivo; `received` guarda os já gravados, para o
/// chamador apagá-los se o envio falhar depois.
async fn receive(
    jobs: &Jobs,
    multipart: &mut Multipart,
    received: &mut Vec<Job>,
) -> Result<(), UploadError> {
    let (mut transcribe, mut compress) = (false, false);
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| UploadError::Interrupted)?
    {
        match field.name().unwrap_or_default() {
            "transcrever" => transcribe = true,
            "compactar" => compress = true,
            "arquivo" => {
                // Input sem arquivo escolhido manda um campo com nome vazio.
                let Some(name) = field.file_name().and_then(display_name) else {
                    continue;
                };
                let (media, ext) = Media::detect(&name).ok_or(UploadError::Format)?;
                // No formulário, as opções vêm antes do arquivo: recusa antes de gravar.
                let options = Options::new(transcribe, compress).ok_or(UploadError::NoOptions)?;
                let id = JobId::generate();
                let input = jobs.upload_path(id, ext);
                let size = save(&mut field, &input).await?;
                received.push(Job {
                    id,
                    name,
                    media,
                    size,
                    options,
                    input,
                    state: JobState::Queued,
                });
            }
            _ => {}
        }
    }
    if received.is_empty() {
        return Err(UploadError::NoFile);
    }
    Ok(())
}

/// Grava o campo em pedaços, sem juntar o arquivo na memória.
async fn save(field: &mut Field<'_>, path: &FsPath) -> Result<u64, UploadError> {
    let mut partial = Partial(Some(path.to_path_buf()));
    let mut out = BufWriter::with_capacity(1 << 20, tokio::fs::File::create(path).await?);
    let mut size = 0;
    let result = async {
        while let Some(chunk) = field.chunk().await.map_err(|_| UploadError::Interrupted)? {
            out.write_all(&chunk).await?;
            size += chunk.len() as u64;
        }
        out.flush().await?;
        Ok(())
    }
    .await;
    if result.is_ok() {
        partial.0 = None;
    } else {
        // Termina a escrita pendente para o arquivo fechar antes de ser apagado.
        let _ = out.flush().await;
    }
    result.map(|()| size)
}

/// Apaga o arquivo parcial se o envio não chegar ao fim, inclusive quando o servidor
/// encerra e descarta a requisição no meio.
struct Partial(Option<PathBuf>);

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Nome exibido: só o último componente do nome enviado (navegadores antigos mandavam o
/// caminho inteiro).
fn display_name(raw: &str) -> Option<String> {
    let name = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Tamanho em unidades binárias, com vírgula decimal: "1,5 KB", "312 MB".
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    let text = if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    };
    let text = text.strip_suffix(".0").unwrap_or(&text).replace('.', ",");
    format!("{text} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::Stage;

    #[test]
    fn mensagem_e_accept_vem_da_lista_de_formatos() {
        assert_eq!(
            format_error(),
            "Formato não suportado. Use MP4, MOV, MKV, MP3, M4A, WAV ou OGG."
        );
        assert_eq!(accept(), ".mp4,.mov,.mkv,.mp3,.m4a,.wav,.ogg");
    }

    #[test]
    fn nome_exibido_e_so_o_arquivo() {
        assert_eq!(display_name("reuniao.mp4").as_deref(), Some("reuniao.mp4"));
        assert_eq!(
            display_name(r"C:\Users\x\Vídeos\aula.mkv").as_deref(),
            Some("aula.mkv")
        );
        assert_eq!(display_name("../../etc/a.wav").as_deref(), Some("a.wav"));
        assert_eq!(display_name(""), None);
        assert_eq!(display_name("  "), None);
    }

    #[test]
    fn tamanho_legivel() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1,5 KB");
        assert_eq!(format_size(2048), "2 KB");
        assert_eq!(format_size(312 * 1024 * 1024), "312 MB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2 GB");
        assert_eq!(format_size(5_500_000_000), "5,1 GB");
    }

    fn job(state: JobState) -> Job {
        Job {
            id: "01JAZ5Y3V8K9M2N4P6Q7R8S9T0".parse().unwrap(),
            name: "Reunião <equipe>.mp4".into(),
            media: Media::Video,
            size: 312 * 1024 * 1024,
            options: Options::new(true, false).unwrap(),
            input: "nada".into(),
            state,
        }
    }

    #[derive(Template)]
    #[template(path = "partials/job_card.html")]
    struct CardTemplate<'a> {
        job: &'a Card,
    }

    #[test]
    fn card_em_cada_estado() {
        let running = |canceling| JobState::Running {
            stage: Stage::Transcribing,
            pct: 42,
            canceling,
        };
        let states = [
            ("na_fila", JobState::Queued),
            ("rodando", running(false)),
            ("cancelando", running(true)),
            ("concluido", JobState::Done { outputs: vec![] }),
            (
                "falhou",
                JobState::Failed {
                    message: "Não foi possível processar este arquivo.".into(),
                },
            ),
            ("cancelado", JobState::Canceled),
        ];
        for (name, state) in states {
            let html = CardTemplate {
                job: &Card::new(&job(state)),
            }
            .render()
            .unwrap();
            insta::assert_snapshot!(name, html);
        }
    }
}
