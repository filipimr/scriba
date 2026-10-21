//! A gravação ao vivo na tela: Gravar e Parar, o estado pelo polling, os medidores e o tempo
//! pelo SSE, e os Ajustes (microfone e saída). As respostas são fragmentos HTML, quase todos
//! fora de banda: a `recording-bar` (em todas as páginas) e, com `?painel=1` (a página
//! inicial), o `record-panel` e o card do job novo. A barra é quem faz o polling.

use std::convert::Infallible;
use std::time::Duration;

use askama::Template;
use axum::extract::{Form, RawQuery, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;

use super::AppState;
use super::jobs::{Card, has_param, with_error_toast};
use crate::capture::{Choice, Device, Devices, Error, Status};
use crate::config::Config;
use crate::jobs::JobId;

/// Resposta que encerra o polling do htmx.
const STOP_POLLING: u16 = 286;

/// O intervalo dos eventos do SSE (~10 por segundo).
pub const LEVELS_EVERY: Duration = Duration::from_millis(100);

/// O `record-panel`.
pub struct PanelView {
    pub recording: bool,
    /// Vai fora de banda (troca o `#gravar` da página).
    pub oob: bool,
    pub elapsed: String,
    /// "Você" e "Outros", de 0 a 1 na escala do medidor.
    pub levels: [String; 2],
    pub mic_silent: bool,
    pub suspended: bool,
    /// O motivo de a gravação não ter começado.
    pub error: Option<String>,
}

impl PanelView {
    pub fn new(status: &Status, oob: bool, error: Option<String>) -> Self {
        match status {
            Status::Recording {
                elapsed_ms,
                levels,
                mic_silent,
                suspended,
                ..
            } => Self {
                recording: true,
                oob,
                elapsed: clock(*elapsed_ms),
                levels: levels.map(|l| format!("{:.2}", meter(l))),
                mic_silent: *mic_silent,
                suspended: *suspended,
                error,
            },
            Status::Idle { .. } => Self {
                recording: false,
                oob,
                elapsed: clock(0),
                levels: ["0.00".into(), "0.00".into()],
                mic_silent: false,
                suspended: false,
                error,
            },
        }
    }
}

/// A `recording-bar` enquanto grava.
pub struct BarView {
    pub id: String,
    pub elapsed: String,
    /// A página tem o painel: o polling e o Parar pedem as partes dele.
    pub painel: bool,
    pub oob: bool,
}

/// O que a resposta faz com a barra.
pub enum Bar {
    /// Não mexe.
    Skip,
    Show(BarView),
    /// O lugar vazio (sem gravação); `true` = fora de banda.
    Empty(bool),
}

impl Bar {
    pub fn new(status: &Status, painel: bool, oob: bool) -> Self {
        match status {
            Status::Recording { id, elapsed_ms, .. } => Self::Show(BarView {
                id: id.to_string(),
                elapsed: clock(*elapsed_ms),
                painel,
                oob,
            }),
            Status::Idle { .. } => Self::Empty(oob),
        }
    }
}

#[derive(Template)]
#[template(path = "partials/recording_bar.html")]
struct BarTemplate {
    bar: Bar,
}

/// A barra da página, renderizada no servidor para o `base.html`.
pub fn bar_html(state: &AppState, painel: bool) -> String {
    let bar = Bar::new(&state.recorder.status(), painel, false);
    BarTemplate { bar }.render().unwrap_or_default()
}

#[derive(Template)]
#[template(path = "partials/record.html")]
pub struct Fragment {
    pub bar: Bar,
    /// O painel inteiro, fora de banda.
    pub panel: Option<PanelView>,
    /// Só o tempo e os avisos do painel, fora de banda (o polling não fecha os Ajustes).
    pub parts: Option<PanelView>,
    /// O card do job da gravação, no topo de `#jobs`.
    pub card: Option<Card>,
    pub notice: Option<String>,
}

impl Fragment {
    fn new(bar: Bar) -> Self {
        Self {
            bar,
            panel: None,
            parts: None,
            card: None,
            notice: None,
        }
    }

    fn respond(self, status: StatusCode) -> Response {
        match self.render() {
            Ok(html) => (status, Html(html)).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }
}

/// `00:12:05`.
pub fn clock(ms: u64) -> String {
    let secs = ms / 1_000;
    format!("{:02}:{:02}:{:02}", secs / 3_600, secs / 60 % 60, secs % 60)
}

/// O nível RMS na escala do medidor: −60 dBFS (ou menos) é vazio, 0 dBFS é cheio.
pub fn meter(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    ((20.0 * rms.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn painel(query: Option<&str>) -> bool {
    has_param(query, "painel=1")
}

fn param<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?
        .split('&')
        .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
}

/// O card do job que a gravação virou, se a página tem a lista.
fn card(state: &AppState, id: JobId, painel: bool) -> Option<Card> {
    painel
        .then(|| state.jobs.get(id))
        .flatten()
        .map(|job| Card::new(&job))
}

/// Os dispositivos dos Ajustes, numa thread de bloqueio (o cpal enumera pelo COM).
async fn list_devices(state: &AppState) -> Option<Devices> {
    let recorder = state.recorder.clone();
    match tokio::task::spawn_blocking(move || recorder.devices()).await {
        Ok(Ok(devices)) => Some(devices),
        Ok(Err(e)) => {
            eprintln!("gravação: não listei os dispositivos: {e}");
            None
        }
        Err(_) => None,
    }
}

/// O que os Ajustes pedem; um id salvo que sumiu cai no padrão do Windows. Sem a lista, vale
/// o que está salvo.
pub fn choice(config: &Config, devices: Option<&Devices>) -> Choice {
    let keep = |id: &Option<String>, list: Option<&Vec<Device>>| {
        id.clone()
            .filter(|id| list.is_none_or(|l| l.iter().any(|d| &d.id == id)))
    };
    Choice {
        mic: keep(&config.mic, devices.map(|d| &d.inputs)),
        output: keep(&config.output, devices.map(|d| &d.outputs)),
    }
}

/// `POST /api/record/start`: grava nos dispositivos dos Ajustes. 409 com uma gravação em
/// andamento; 507 com pouco disco; 422 sem microfone ou saída, ou com o microfone bloqueado.
/// O erro aparece no painel (ou num toast, numa página sem ele).
pub async fn start(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let painel = painel(query.as_deref());
    let devices = list_devices(&state).await;
    let choice = choice(&state.config(), devices.as_ref());
    let recorder = state.recorder.clone();
    let result = tokio::task::spawn_blocking(move || recorder.start(&choice))
        .await
        .unwrap_or(Err(Error::Files));
    match result {
        Ok(status) => {
            let mut fragment = Fragment::new(Bar::new(&status, painel, true));
            fragment.panel = painel.then(|| PanelView::new(&status, true, None));
            fragment.notice = Some("Gravação iniciada.".into());
            fragment.respond(StatusCode::OK)
        }
        Err(Error::Busy) => (StatusCode::CONFLICT, Error::Busy.to_string()).into_response(),
        Err(e) => {
            let code = match e {
                Error::Disk | Error::Files => StatusCode::INSUFFICIENT_STORAGE,
                _ => StatusCode::UNPROCESSABLE_ENTITY,
            };
            let message = e.to_string();
            if painel {
                let mut fragment = Fragment::new(Bar::Skip);
                let idle = state.recorder.status();
                fragment.panel = Some(PanelView::new(&idle, true, Some(message)));
                fragment.respond(code)
            } else {
                with_error_toast(Fragment::new(Bar::Skip).respond(code), &message)
            }
        }
    }
}

/// `POST /api/record/stop`: fecha as trilhas e põe a gravação na fila; 409 sem gravação.
pub async fn stop(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let painel = painel(query.as_deref());
    let recorder = state.recorder.clone();
    let result = tokio::task::spawn_blocking(move || recorder.stop())
        .await
        .unwrap_or(Err(Error::NotRecording));
    match result {
        Ok(recording) => {
            let idle = state.recorder.status();
            let mut fragment = Fragment::new(Bar::Empty(true));
            fragment.panel = painel.then(|| PanelView::new(&idle, true, None));
            fragment.card = card(&state, recording.id, painel);
            fragment.notice = Some("Gravação parada. Transcrevendo.".into());
            let response = fragment.respond(StatusCode::OK);
            // A gravação acabou sozinha um instante antes do Parar.
            match recording.end.message() {
                Some(message) => with_error_toast(response, message),
                None => response,
            }
        }
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

/// `GET /api/record/status?id=…`: o polling da barra. Gravando, 200 com a barra (e o tempo e
/// os avisos do painel); parada, 286 com o lugar vazio, o painel parado e, se a gravação `id`
/// acabou, o card dela e o motivo do fim automático num toast.
pub async fn status(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let query = query.as_deref();
    let painel = painel(query);
    let status = state.recorder.status();
    let mut fragment = Fragment::new(Bar::new(&status, painel, false));
    let Status::Idle { last } = status else {
        fragment.parts = painel.then(|| PanelView::new(&status, true, None));
        return fragment.respond(StatusCode::OK);
    };
    fragment.panel = painel.then(|| PanelView::new(&status, true, None));
    let watched = param(query, "id").and_then(|id| id.parse::<JobId>().ok());
    let mut toast = None;
    if let Some((id, end)) = last
        && watched == Some(id)
    {
        fragment.card = card(&state, id, painel);
        fragment.notice = Some("Gravação parada. Transcrevendo.".into());
        toast = end.message();
    }
    let code = StatusCode::from_u16(STOP_POLLING).unwrap_or(StatusCode::OK);
    let response = fragment.respond(code);
    match toast {
        Some(message) => with_error_toast(response, message),
        None => response,
    }
}

/// Um evento dos medidores: o tempo e os dois níveis já na escala do medidor.
fn level_event(elapsed_ms: u64, levels: [f32; 2]) -> Event {
    Event::default().data(format!(
        r#"{{"tempo":"{}","niveis":[{:.3},{:.3}]}}"#,
        clock(elapsed_ms),
        meter(levels[0]),
        meter(levels[1])
    ))
}

/// `GET /api/record/levels`: SSE com o tempo e os níveis a cada [`LEVELS_EVERY`]. Termina
/// com o evento `fim` quando a gravação para ou o app fecha (o `EventSource` não reconecta).
pub async fn levels(State(state): State<AppState>) -> Response {
    let stream = futures_util::stream::unfold(Some(true), move |step| {
        let state = state.clone();
        async move {
            let first = step?;
            if !first {
                tokio::time::sleep(LEVELS_EVERY).await;
            }
            let event = match state.recorder.status() {
                Status::Recording {
                    elapsed_ms, levels, ..
                } if !state.is_closing() => (level_event(elapsed_ms, levels), Some(false)),
                _ => (Event::default().event("fim").data("fim"), None),
            };
            Some((Ok::<_, Infallible>(event.0), event.1))
        }
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Uma opção de um `device-select`.
pub struct OptionView {
    pub id: String,
    pub label: String,
    pub selected: bool,
}

pub struct SelectView {
    pub name: &'static str,
    pub label: &'static str,
    pub options: Vec<OptionView>,
    /// O dispositivo que vai gravar é Bluetooth (só o microfone avisa).
    pub bluetooth: bool,
}

pub struct DevicesView {
    pub recording: bool,
    pub selects: Vec<SelectView>,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "partials/devices.html")]
pub struct DevicesTemplate {
    pub view: DevicesView,
}

fn select(
    name: &'static str,
    label: &'static str,
    list: &[Device],
    chosen: Option<&str>,
    warn_bluetooth: bool,
) -> SelectView {
    let default = list.iter().find(|d| d.default);
    let chosen = chosen.and_then(|id| list.iter().find(|d| d.id == id));
    let first = match default {
        Some(d) => format!("Padrão do Windows ({})", d.name),
        None => "Padrão do Windows".into(),
    };
    let mut options = vec![OptionView {
        id: String::new(),
        label: first,
        selected: chosen.is_none(),
    }];
    options.extend(list.iter().map(|d| OptionView {
        id: d.id.clone(),
        label: if d.default {
            format!("{} (padrão)", d.name)
        } else {
            d.name.clone()
        },
        selected: chosen.is_some_and(|c| c.id == d.id),
    }));
    SelectView {
        name,
        label,
        options,
        bluetooth: warn_bluetooth && chosen.or(default).is_some_and(|d| d.bluetooth),
    }
}

/// Os dois selects, com a escolha salva (um id que sumiu volta ao padrão).
pub fn devices_view(config: &Config, devices: Option<&Devices>, recording: bool) -> DevicesView {
    let empty = Devices::default();
    let list = devices.unwrap_or(&empty);
    let choice = choice(config, devices);
    DevicesView {
        recording,
        selects: vec![
            select(
                "mic",
                "Microfone",
                &list.inputs,
                choice.mic.as_deref(),
                true,
            ),
            select(
                "output",
                "Saída do som",
                &list.outputs,
                choice.output.as_deref(),
                false,
            ),
        ],
        error: devices
            .is_none()
            .then(|| "Não consegui listar os dispositivos de som.".into()),
    }
}

async fn render_devices(state: &AppState, code: StatusCode) -> Response {
    let devices = list_devices(state).await;
    let recording = matches!(state.recorder.status(), Status::Recording { .. });
    let view = devices_view(&state.config(), devices.as_ref(), recording);
    match (DevicesTemplate { view }).render() {
        Ok(html) => (code, Html(html)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// `GET /api/record/devices`: os Ajustes, com a lista de agora.
pub async fn devices(State(state): State<AppState>) -> Response {
    render_devices(&state, StatusCode::OK).await
}

#[derive(Deserialize)]
pub struct DeviceForm {
    #[serde(default)]
    mic: String,
    #[serde(default)]
    output: String,
}

/// `POST /api/record/devices`: salva a escolha no `config.json` (vazio = o padrão do
/// Windows); vale na próxima gravação. 409 enquanto grava.
pub async fn choose(State(state): State<AppState>, Form(form): Form<DeviceForm>) -> Response {
    if matches!(state.recorder.status(), Status::Recording { .. }) {
        return (
            StatusCode::CONFLICT,
            "Espere a gravação parar para trocar os dispositivos.",
        )
            .into_response();
    }
    let some = |s: String| (!s.is_empty()).then_some(s);
    state.update_config(|c| {
        c.mic = some(form.mic);
        c.output = some(form.output);
    });
    render_devices(&state, StatusCode::OK).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::LocalTime;

    #[test]
    fn relogio_e_medidor() {
        assert_eq!(clock(0), "00:00:00");
        assert_eq!(clock(61_999), "00:01:01");
        assert_eq!(clock(12 * 3_600_000 + 5_000), "12:00:05");
        assert_eq!(meter(0.0), 0.0);
        assert_eq!(meter(1.0), 1.0);
        assert!((meter(0.001) - 0.0).abs() < 1e-6);
        assert!((meter(0.1) - 2.0 / 3.0).abs() < 1e-3);
    }

    fn device(id: &str, name: &str, default: bool, bluetooth: bool) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            default,
            bluetooth,
        }
    }

    fn list() -> Devices {
        Devices {
            inputs: vec![
                device("mic", "Microfone (Realtek)", true, false),
                device("bt", "Fones AirPods", false, true),
            ],
            outputs: vec![device("alto", "Alto-falantes", true, false)],
        }
    }

    #[test]
    fn id_salvo_que_sumiu_cai_no_padrao() {
        let config = Config {
            mic: Some("sumiu".into()),
            output: Some("alto".into()),
            ..Config::default()
        };
        let c = choice(&config, Some(&list()));
        assert_eq!(c.mic, None);
        assert_eq!(c.output.as_deref(), Some("alto"));
        // Sem a lista, vale o salvo.
        assert_eq!(choice(&config, None).mic.as_deref(), Some("sumiu"));
    }

    #[test]
    fn bluetooth_avisa_pelo_escolhido_ou_pelo_padrao() {
        let bt = Config {
            mic: Some("bt".into()),
            ..Config::default()
        };
        let view = devices_view(&bt, Some(&list()), false);
        assert!(view.selects[0].bluetooth);
        assert!(!view.selects[1].bluetooth);
        assert!(view.selects[0].options[2].selected);
        assert_eq!(
            view.selects[0].options[0].label,
            "Padrão do Windows (Microfone (Realtek))"
        );
        assert_eq!(
            view.selects[0].options[1].label,
            "Microfone (Realtek) (padrão)"
        );
        let mut devices = list();
        devices.inputs[0].default = false;
        devices.inputs[1].default = true;
        let view = devices_view(&Config::default(), Some(&devices), false);
        assert!(view.selects[0].bluetooth, "o padrão é o fone");
        assert!(view.selects[0].options[0].selected);
    }

    fn recording() -> Status {
        Status::Recording {
            id: "01JAZ5Y3V8K9M2N4P6Q7R8S9T0".parse().unwrap(),
            created: LocalTime {
                year: 2026,
                month: 10,
                day: 21,
                hour: 14,
                minute: 30,
                second: 0,
                offset_minutes: -180,
            },
            elapsed_ms: 754_321,
            levels: [0.1, 0.01],
            mic_silent: true,
            suspended: false,
            mic: "Microfone (Realtek)".into(),
            output: "Alto-falantes".into(),
        }
    }

    #[test]
    fn painel_e_barra_em_cada_estado() {
        let idle = Status::Idle { last: None };
        let cases = [
            ("painel_parado", PanelView::new(&idle, false, None)),
            ("painel_gravando", PanelView::new(&recording(), false, None)),
            (
                "painel_erro",
                PanelView::new(&idle, false, Some(Error::NoMic.to_string())),
            ),
        ];
        #[derive(Template)]
        #[template(path = "partials/record_panel.html")]
        struct PanelTemplate {
            panel: PanelView,
        }
        for (name, panel) in cases {
            let html = PanelTemplate { panel }.render().unwrap();
            insta::assert_snapshot!(name, html);
        }
        let bar = BarTemplate {
            bar: Bar::new(&recording(), true, false),
        }
        .render()
        .unwrap();
        insta::assert_snapshot!("barra_gravando", bar);
    }

    #[test]
    fn ajustes() {
        let bt = Config {
            mic: Some("bt".into()),
            ..Config::default()
        };
        for (name, view) in [
            ("ajustes", devices_view(&bt, Some(&list()), false)),
            (
                "ajustes_gravando",
                devices_view(&Config::default(), Some(&list()), true),
            ),
            (
                "ajustes_sem_lista",
                devices_view(&Config::default(), None, false),
            ),
        ] {
            let html = DevicesTemplate { view }.render().unwrap();
            insta::assert_snapshot!(name, html);
        }
    }
}
