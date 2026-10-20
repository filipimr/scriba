//! Servidor HTTP local (Axum em 127.0.0.1) que serve a UI.

mod assets;
mod jobs;
mod pages;
pub mod security;

use std::future::{Future, IntoFuture};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};

use crate::config::{Config, Paths, Theme};
use crate::jobs::Jobs;

/// Quanto o encerramento espera as requisições em andamento (um upload longo, por exemplo).
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub token: String,
    pub instance_id: String,
    pub port: u16,
    pub paths: Paths,
    pub jobs: Jobs,
    config: Mutex<Config>,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(token: String, instance_id: String, port: u16, paths: Paths, jobs: Jobs) -> Self {
        let config = Config::load(&paths.config_file());
        Self(Arc::new(Inner {
            token,
            instance_id,
            port,
            paths,
            jobs,
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
        // Só o envio de arquivos aceita corpo grande: ele vai em streaming para o disco.
        .route(
            "/api/jobs",
            get(jobs::list)
                .post(jobs::create)
                .layer(DefaultBodyLimit::disable()),
        )
        .route("/api/jobs/{id}", get(jobs::show).delete(jobs::remove))
        .route("/api/jobs/{id}/cancel", post(jobs::cancel))
        .route("/api/jobs/{id}/retry", post(jobs::retry))
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

/// Serve até `shutdown` completar. As requisições em andamento têm até `grace` para
/// terminar; depois disso são abandonadas (um upload parado não trava o encerramento).
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
    grace: Duration,
) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    let (signaled, deadline) = tokio::sync::oneshot::channel::<()>();
    let server = axum::serve(listener, router(state)).with_graceful_shutdown(async move {
        shutdown.await;
        let _ = signaled.send(());
    });
    tokio::select! {
        result = server.into_future() => result,
        () = async move {
            match deadline.await {
                Ok(()) => tokio::time::sleep(grace).await,
                // O servidor terminou sozinho (erro): o outro braço responde.
                Err(_) => std::future::pending().await,
            }
        } => Ok(()),
    }
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
