//! Um caminho só de encerramento: o Ctrl+C, o "Sair" da bandeja, o fim da sessão do Windows
//! (`WM_ENDSESSION`) e a rota `POST /api/app/sair` pedem o mesmo `Shutdown`, e o `app::run`
//! fecha tudo na mesma ordem.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::Notify;

/// Quem pediu para fechar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    CtrlC,
    /// "Sair" no menu da bandeja.
    Tray,
    /// Logoff ou desligamento: a gravação fecha primeiro, e o servidor não espera a folga.
    EndSession,
    /// `POST /api/app/sair`.
    Route,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Self::CtrlC => "Ctrl+C",
            Self::Tray => "Sair na bandeja",
            Self::EndSession => "fim da sessão do Windows",
            Self::Route => "rota de saída",
        }
    }
}

#[derive(Default)]
struct State {
    /// O primeiro pedido vale; os outros só repetem.
    reason: Option<Reason>,
    finished: bool,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    changed: Condvar,
    notify: Notify,
}

/// Handle clonável do pedido de encerramento.
#[derive(Clone, Default)]
pub struct Shutdown(Arc<Inner>);

impl Shutdown {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pede o encerramento; vale de qualquer thread, antes ou depois de alguém esperar.
    pub fn request(&self, reason: Reason) {
        let mut state = self.lock();
        if state.reason.is_none() {
            state.reason = Some(reason);
            // `notify_one` guarda a vez se ninguém espera ainda.
            self.0.notify.notify_one();
        }
    }

    pub fn reason(&self) -> Option<Reason> {
        self.lock().reason
    }

    /// Espera o primeiro pedido.
    pub async fn requested(&self) -> Reason {
        loop {
            if let Some(reason) = self.reason() {
                return reason;
            }
            self.0.notify.notified().await;
        }
    }

    /// O app terminou de fechar: a gravação, a fila e o `instance.json`.
    pub fn finish(&self) {
        self.lock().finished = true;
        self.0.changed.notify_all();
    }

    /// Espera o app terminar de fechar (o `WM_ENDSESSION` só volta depois disso); `false`
    /// se o prazo passou.
    pub fn wait_finished(&self, timeout: Duration) -> bool {
        let state = self.lock();
        let (state, _) = self
            .0
            .changed
            .wait_timeout_while(state, timeout, |s| !s.finished)
            .unwrap_or_else(PoisonError::into_inner);
        state.finished
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn o_primeiro_pedido_vale_mesmo_antes_de_esperar() {
        let shutdown = Shutdown::new();
        assert_eq!(shutdown.reason(), None);
        shutdown.request(Reason::Tray);
        shutdown.request(Reason::CtrlC);
        assert_eq!(shutdown.requested().await, Reason::Tray);
        assert_eq!(shutdown.reason(), Some(Reason::Tray));
    }

    #[tokio::test]
    async fn pedido_de_outra_thread_acorda_quem_espera() {
        let shutdown = Shutdown::new();
        let other = shutdown.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            other.request(Reason::EndSession);
        });
        assert_eq!(shutdown.requested().await, Reason::EndSession);
    }

    #[test]
    fn espera_o_fim_com_prazo() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.wait_finished(Duration::from_millis(10)));
        let other = shutdown.clone();
        let t = std::thread::spawn(move || other.wait_finished(Duration::from_secs(10)));
        std::thread::sleep(Duration::from_millis(20));
        shutdown.finish();
        assert!(t.join().unwrap());
        assert!(shutdown.wait_finished(Duration::ZERO));
    }

    #[test]
    fn rotulos() {
        for r in [
            Reason::CtrlC,
            Reason::Tray,
            Reason::EndSession,
            Reason::Route,
        ] {
            assert!(!r.label().is_empty());
        }
    }
}
