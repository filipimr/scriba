//! Envio de arquivos em streaming para o disco e as rotas da fila, que respondem fragmentos
//! HTML para o htmx: o card de cada job e, no fim dele, um aviso para leitores de tela.

use std::io;
use std::path::{Path as FsPath, PathBuf};

use askama::Template;
use axum::extract::multipart::Field;
use axum::extract::{Multipart, Path, RawQuery, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use tokio::io::{AsyncWriteExt, BufWriter};

use super::AppState;
use super::reader::{Kind, download_base, download_name, folder_label};
use crate::clock::LocalTime;
use crate::jobs::{
    Audio, Estimate, FORMATS, Failure, Job, JobId, Jobs, Media, PANIC_MESSAGE, Probe, RemoveError,
    RetryError, State as JobState,
};

/// Resposta que encerra o polling do htmx.
const STOP_POLLING: u16 = 286;

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
    /// "2 min"; `None` quando o arquivo não informa.
    pub duration: Option<String>,
    pub size: String,
    /// Data e hora do envio, num `<span>` próprio (a referência visual a mascara).
    pub date: String,
    /// Só na fila: "Cerca de 4 min na GPU".
    pub estimate: Option<String>,
    pub badge: &'static str,
    /// Sufixo da variante do `status-badge` (`active`, `success`, `error`); vazio no neutro.
    pub badge_variant: &'static str,
    /// O card faz polling: o job está ativo ou o áudio está sendo guardado.
    pub polling: bool,
    /// O que o botão de ação faz.
    pub action: Action,
    pub canceling: bool,
    /// Etapa com a porcentagem, só enquanto roda.
    pub stage: Option<String>,
    pub pct: u8,
    pub error: Option<String>,
    /// Falhou: oferece "Tentar de novo".
    pub retry: bool,
    /// Concluído: as ações do resultado e a pasta que o Remover manda para a Lixeira.
    pub done: Option<Done>,
}

/// O botão à direita do card, que mantém o id entre os estados.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Na fila ou rodando.
    CancelJob,
    /// Guardando o áudio.
    CancelAudio,
    /// A janela "Salvar como" está aberta: Remover fica inativo.
    Wait,
    Remove,
}

/// O que o card concluído oferece.
pub struct Done {
    /// Nome da pasta da saída, no texto do `confirm-dialog`.
    pub folder: String,
    /// Nomes sugeridos dos downloads ("Reunião - transcrição.txt").
    pub txt_name: String,
    pub md_name: String,
    pub audio: AudioView,
}

/// "Guardar o áudio?" no card concluído. Descartado: nada aparece.
#[derive(Default)]
pub struct AudioView {
    /// A pergunta, com "Guardar áudio" e "Descartar".
    pub ask: bool,
    /// "Escolhendo onde guardar…", "Guardando áudio… 42%", "Cancelando…".
    pub stage: Option<String>,
    /// A barra, só enquanto grava.
    pub pct: Option<u8>,
    /// O caminho do áudio guardado.
    pub saved: Option<String>,
}

impl AudioView {
    fn new(audio: &Audio) -> Self {
        match audio {
            Audio::Pending { .. } => Self {
                ask: true,
                ..Self::default()
            },
            Audio::Choosing => Self {
                stage: Some("Escolhendo onde guardar…".into()),
                ..Self::default()
            },
            Audio::Saving {
                canceling: true,
                pct,
            } => Self {
                stage: Some("Cancelando…".into()),
                pct: Some(*pct),
                ..Self::default()
            },
            Audio::Saving { pct, .. } => Self {
                stage: Some(format!("Guardando áudio… {pct}%")),
                pct: Some(*pct),
                ..Self::default()
            },
            Audio::Saved { path } => Self {
                saved: Some(path.display().to_string()),
                ..Self::default()
            },
            Audio::Discarded => Self::default(),
        }
    }

    /// Há algo a mostrar.
    pub fn visible(&self) -> bool {
        self.ask || self.stage.is_some() || self.saved.is_some()
    }
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
            JobState::Done {
                audio: Audio::Saving { canceling, .. },
                ..
            } => (None, 0, *canceling),
            _ => (None, 0, false),
        };
        let action = match &job.state {
            state if !state.is_terminal() => Action::CancelJob,
            JobState::Done {
                audio: Audio::Saving { .. },
                ..
            } => Action::CancelAudio,
            JobState::Done {
                audio: Audio::Choosing,
                ..
            } => Action::Wait,
            _ => Action::Remove,
        };
        Self {
            id: job.id.to_string(),
            name: job.name.clone(),
            video: job.media == Media::Video,
            duration: format_duration(job.duration_ms),
            size: format_size(job.size),
            date: job.created.display(),
            estimate: match (&job.state, job.estimate) {
                (JobState::Queued, Some(e)) => Some(format_estimate(e)),
                _ => None,
            },
            badge,
            badge_variant,
            polling: polls(job),
            action,
            canceling,
            stage,
            pct,
            error: match &job.state {
                JobState::Failed { message } => Some(message.clone()),
                _ => None,
            },
            retry: matches!(job.state, JobState::Failed { .. }),
            done: match &job.state {
                JobState::Done { outputs, audio } => Some(Done {
                    folder: folder_label(&outputs.dir),
                    txt_name: download_name(&download_base(job), Kind::Txt),
                    md_name: download_name(&download_base(job), Kind::Md),
                    audio: AudioView::new(audio),
                }),
                _ => None,
            },
        }
    }
}

/// O card acompanha o job pelo polling: ativo, ou guardando o áudio.
fn polls(job: &Job) -> bool {
    match &job.state {
        JobState::Done { audio, .. } => audio.busy(),
        state => !state.is_terminal(),
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

pub(super) fn fragment(status: StatusCode, jobs: Vec<Card>, notice: Option<String>) -> Response {
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

pub(super) fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Job não encontrado.").into_response()
}

pub async fn list(State(state): State<AppState>) -> Response {
    let cards = state.jobs.list().iter().map(Card::new).collect();
    fragment(StatusCode::OK, cards, None)
}

/// Polling do card: enquanto o job está ativo (ou guardando o áudio), 200; no fim, 286 com o
/// card final. `?audio=1` é o polling da gravação do áudio: o fim avisa "Áudio de X
/// guardado." ou traz o erro para o toast (evento `scriba-erro`).
pub async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(job) = id.parse().ok().and_then(|id| state.jobs.get(id)) else {
        return not_found();
    };
    let card = vec![Card::new(&job)];
    if polls(&job) {
        return fragment(StatusCode::OK, card, None);
    }
    let status = StatusCode::from_u16(STOP_POLLING).unwrap_or(StatusCode::OK);
    if !has_param(query.as_deref(), "audio=1") {
        return fragment(status, card, Some(ended(&job)));
    }
    match &job.state {
        JobState::Done {
            audio: Audio::Saved { .. },
            ..
        } => fragment(
            status,
            card,
            Some(format!("Áudio de {} guardado.", job.name)),
        ),
        JobState::Done {
            audio: Audio::Pending {
                error: Some(message),
            },
            ..
        } => with_error_toast(fragment(status, card, None), message),
        _ => fragment(status, card, None),
    }
}

/// `p` é um dos parâmetros da query (`a=1&b=2`).
pub(super) fn has_param(query: Option<&str>, p: &str) -> bool {
    query.is_some_and(|q| q.split('&').any(|x| x == p))
}

/// Acrescenta o evento `scriba-erro` (HX-Trigger), que o `app.js` mostra num toast.
pub(super) fn with_error_toast(mut response: Response, message: &str) -> Response {
    let event = format!(r#"{{"scriba-erro":{}}}"#, ascii_json(message));
    if let Ok(value) = HeaderValue::from_str(&event) {
        response.headers_mut().insert("HX-Trigger", value);
    }
    response
}

/// A string em JSON só com ASCII (`\uXXXX`): o XHR lê os cabeçalhos como Latin-1, e "Não"
/// chegaria como "NÃ£o".
pub fn ascii_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
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

/// Corpo vazio: o htmx troca o card por nada. Um job concluído só sai com `?confirm=1`
/// (o `confirm-dialog` da tela), e a pasta da saída vai para a Lixeira.
pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let Ok(id) = id.parse::<JobId>() else {
        return not_found();
    };
    let confirmed = has_param(query.as_deref(), "confirm=1");
    // O shell da Lixeira bloqueia.
    let jobs = state.jobs.clone();
    let result = tokio::task::spawn_blocking(move || jobs.remove(id, confirmed))
        .await
        .unwrap_or(Err(RemoveError::Trash));
    match result {
        Ok(()) => StatusCode::OK.into_response(),
        Err(RemoveError::NotFound) => not_found(),
        Err(RemoveError::Active) => {
            (StatusCode::CONFLICT, "Cancele o job antes de remover.").into_response()
        }
        Err(RemoveError::AudioBusy) => {
            (StatusCode::CONFLICT, "Cancele o áudio antes de remover.").into_response()
        }
        Err(RemoveError::NeedsConfirm) => (
            StatusCode::CONFLICT,
            "Confirme para mandar a transcrição para a Lixeira.",
        )
            .into_response(),
        Err(RemoveError::Trash) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Não consegui mandar a pasta para a Lixeira.",
        )
            .into_response(),
    }
}

/// "Tentar de novo": o job volta para a fila e o card, ativo, retoma o polling.
pub async fn retry(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<JobId>() else {
        return not_found();
    };
    match state.jobs.retry(id) {
        Ok(job) => {
            let notice = format!("{} na fila.", job.name);
            fragment(StatusCode::OK, vec![Card::new(&job)], Some(notice))
        }
        Err(RetryError::NotFound) => not_found(),
        Err(RetryError::NotFailed) => (
            StatusCode::CONFLICT,
            "Só um job que falhou pode ser tentado de novo.",
        )
            .into_response(),
    }
}

/// `POST /api/jobs` (multipart): um ou mais campos `arquivo`, gravados em streaming em
/// `uploads/` e lidos pelo probe. Cada arquivo vira um job.
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
    /// O probe recusou o arquivo (sem áudio, não é mídia), com a mensagem dele.
    Rejected(String),
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
        UploadError::Rejected(message) => (StatusCode::UNPROCESSABLE_ENTITY, message.clone()),
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
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| UploadError::Interrupted)?
    {
        if field.name() != Some("arquivo") {
            continue;
        }
        // Input sem arquivo escolhido manda um campo com nome vazio.
        let Some(name) = field.file_name().and_then(display_name) else {
            continue;
        };
        // A extensão é o primeiro filtro: recusa antes de gravar.
        let (_, ext) = Media::detect(&name).ok_or(UploadError::Format)?;
        let id = JobId::generate();
        let created = LocalTime::now();
        let input = jobs.upload_path(id, ext);
        let size = save(&mut field, &input).await?;
        let probe = match probe(jobs, &name, &input).await {
            Ok(probe) => probe,
            Err(message) => {
                let _ = tokio::fs::remove_file(&input).await;
                return Err(UploadError::Rejected(message));
            }
        };
        received.push(Job {
            id,
            name,
            media: probe.media,
            size,
            duration_ms: probe.duration_ms,
            created,
            estimate: probe.estimate,
            input,
            state: JobState::Queued,
        });
    }
    if received.is_empty() {
        return Err(UploadError::NoFile);
    }
    Ok(())
}

/// O probe do pipeline numa thread de bloqueio; o erro é a mensagem para a tela.
async fn probe(jobs: &Jobs, name: &str, input: &FsPath) -> Result<Probe, String> {
    let (jobs, name, input) = (jobs.clone(), name.to_string(), input.to_path_buf());
    match tokio::task::spawn_blocking(move || jobs.probe(&name, &input)).await {
        Ok(Ok(probe)) => Ok(probe),
        Ok(Err(Failure::Error(message))) => Err(message),
        Ok(Err(Failure::Canceled)) | Err(_) => Err(PANIC_MESSAGE.into()),
    }
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

/// Duração para o card: "45 s", "12 min", "1 h 05 min" (ao minuto mais próximo); `None`
/// quando desconhecida.
pub fn format_duration(ms: u64) -> Option<String> {
    if ms == 0 {
        return None;
    }
    let secs = (ms + 500) / 1_000;
    if secs < 60 {
        return Some(format!("{secs} s"));
    }
    Some(hours_minutes((secs + 30) / 60))
}

fn hours_minutes(minutes: u64) -> String {
    if minutes < 60 {
        format!("{minutes} min")
    } else {
        format!("{} h {:02} min", minutes / 60, minutes % 60)
    }
}

/// "Cerca de 4 min na GPU", "Menos de 1 min no processador": arredonda para cima.
pub fn format_estimate(e: Estimate) -> String {
    let place = if e.gpu { "na GPU" } else { "no processador" };
    if e.secs < 60 {
        format!("Menos de 1 min {place}")
    } else {
        format!("Cerca de {} {place}", hours_minutes(e.secs.div_ceil(60)))
    }
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
    fn duracao_legivel() {
        assert_eq!(format_duration(0), None);
        assert_eq!(format_duration(10_931).as_deref(), Some("11 s"));
        assert_eq!(format_duration(59_400).as_deref(), Some("59 s"));
        assert_eq!(format_duration(89_000).as_deref(), Some("1 min"));
        assert_eq!(format_duration(90_000).as_deref(), Some("2 min"));
        assert_eq!(format_duration(3_570_000).as_deref(), Some("1 h 00 min"));
        assert_eq!(
            format_duration(2 * 3_600_000 + 5 * 60_000).as_deref(),
            Some("2 h 05 min")
        );
    }

    #[test]
    fn estimativa_legivel() {
        let e = |secs, gpu| format_estimate(Estimate { secs, gpu });
        assert_eq!(e(0, true), "Menos de 1 min na GPU");
        assert_eq!(e(59, false), "Menos de 1 min no processador");
        assert_eq!(e(60, true), "Cerca de 1 min na GPU");
        assert_eq!(e(201, true), "Cerca de 4 min na GPU");
        assert_eq!(e(380, false), "Cerca de 7 min no processador");
        assert_eq!(e(4_000, false), "Cerca de 1 h 07 min no processador");
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
            state,
        }
    }

    #[test]
    fn json_do_hx_trigger_so_com_ascii() {
        let message = "Não \"já\" \\ 🎙";
        let json = ascii_json(message);
        assert!(json.is_ascii(), "{json}");
        let backslash = '\\';
        assert!(
            json.starts_with(&format!("\"N{backslash}u00e3o ")),
            "{json}"
        );
        assert!(
            json.ends_with(&format!("{backslash}ud83c{backslash}udf99\"")),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<String>(&json).unwrap(), message);
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
        let done = |audio| JobState::Done {
            outputs: crate::jobs::Outputs {
                dir: "saidas/2026-10-20 1430 - Reunião equipe".into(),
                ..Default::default()
            },
            audio,
        };
        let states = [
            ("na_fila", JobState::Queued),
            ("rodando", running(false)),
            ("cancelando", running(true)),
            ("concluido", done(Audio::PENDING)),
            ("audio_escolhendo", done(Audio::Choosing)),
            (
                "audio_guardando",
                done(Audio::Saving {
                    pct: 42,
                    canceling: false,
                }),
            ),
            (
                "audio_cancelando",
                done(Audio::Saving {
                    pct: 42,
                    canceling: true,
                }),
            ),
            (
                "audio_guardado",
                done(Audio::Saved {
                    path: r"C:\Users\Ana\Música\2026-10-20 1430 - Reunião <equipe>.m4a".into(),
                }),
            ),
            ("audio_descartado", done(Audio::Discarded)),
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
