//! Telas renderizadas no servidor (askama) e a alternância de tema.

use askama::Template;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};

use super::{AppState, assets};
use crate::config::Theme;

/// URLs dos assets usados pelo layout.
pub struct Layout {
    pub css: String,
    pub htmx: String,
    pub js: String,
    pub favicon: String,
    pub logo: String,
    pub version: &'static str,
}

impl Layout {
    fn new() -> Self {
        Self {
            css: assets::url("app.css"),
            htmx: assets::url("vendor/htmx.min.js"),
            js: assets::url("app.js"),
            favicon: assets::url("brand/logo-16.svg"),
            logo: assets::url("brand/logo-white.svg"),
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexPage {
    layout: Layout,
    theme: Theme,
}

#[derive(Template)]
#[template(path = "partials/theme_toggle.html")]
struct ThemeToggle {
    theme: Theme,
}

fn render(t: &impl Template) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub async fn index(State(state): State<AppState>) -> Response {
    render(&IndexPage {
        layout: Layout::new(),
        theme: state.config().theme,
    })
}

/// Avança o ciclo Sistema → Claro → Escuro, salva e devolve o botão novo. O evento
/// `scriba-theme` (HX-Trigger) faz o `app.js` trocar o `data-theme` do `<html>`.
pub async fn toggle_theme(State(state): State<AppState>) -> Response {
    let theme = state.update_config(|c| c.theme = c.theme.next());
    let event = format!(
        r#"{{"scriba-theme":{{"mode":"{}"}}}}"#,
        theme.attr().unwrap_or("system")
    );
    let mut response = render(&ThemeToggle { theme });
    if let Ok(v) = event.parse() {
        response.headers_mut().insert("HX-Trigger", v);
    }
    response
}

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        state.instance_id.clone(),
    )
}
