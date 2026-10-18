//! Servidor HTTP local (Axum em 127.0.0.1) que serve a UI.

mod assets;
mod pages;
pub mod security;

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};

use crate::config::{Config, Paths, Theme};

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub token: String,
    pub instance_id: String,
    pub port: u16,
    pub paths: Paths,
    config: Mutex<Config>,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(token: String, instance_id: String, port: u16, paths: Paths) -> Self {
        let config = Config::load(&paths.config_file());
        Self(Arc::new(Inner {
            token,
            instance_id,
            port,
            paths,
            config: Mutex::new(config),
        }))
    }

    pub fn config(&self) -> Config {
        self.config
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Altera e persiste a configuração; devolve o tema resultante.
    fn update_config(&self, f: impl FnOnce(&mut Config)) -> Theme {
        let mut config = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut config);
        if let Err(e) = config.save(&self.paths.config_file()) {
            eprintln!("não consegui salvar a configuração: {e}");
        }
        config.theme
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(pages::index))
        .route("/api/theme", post(pages::toggle_theme))
        .route("/health", get(pages::health))
        .route("/assets/{*path}", get(assets::serve))
        .layer(from_fn_with_state(state.clone(), security::guard))
        .with_state(state)
}

/// Abre a porta preferida em 127.0.0.1; se estiver ocupada, uma porta livre qualquer.
pub fn bind(preferred: u16) -> std::io::Result<TcpListener> {
    let addr = |port| SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpListener::bind(addr(preferred)).or_else(|_| TcpListener::bind(addr(0)))
}

/// Serve até `shutdown` completar, terminando as requisições em andamento.
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porta_ocupada_cai_para_outra() {
        let busy = bind(0).unwrap();
        let port = busy.local_addr().unwrap().port();
        let other = bind(port).unwrap();
        let addr = other.local_addr().unwrap();
        assert_ne!(addr.port(), port);
        assert!(addr.ip().is_loopback());
    }
}
