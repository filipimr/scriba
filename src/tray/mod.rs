//! O ícone na bandeja. A thread da janela (`win32.rs`) só traduz as mensagens do Windows em
//! [`TrayEvent`] num canal; quem decide o que fazer com cada um é [`run`], sem Win32, com o
//! "abrir" e o "confirmar" atrás de [`Desk`] (fakes nos testes).

mod win32;

use std::sync::mpsc::Receiver;

use crate::capture::Status;
use crate::jobs::{Job, Media, State};
use crate::shutdown::{Reason, Shutdown};

pub use win32::{Options, Tray, WINDOW_CLASS};

/// A dica do ícone.
pub const TOOLTIP: &str = "Scriba";
pub const TOOLTIP_RECORDING: &str = "Scriba — gravando";
/// O menu: o primeiro item é o padrão (em negrito).
pub const MENU_OPEN: &str = "Abrir o Scriba";
pub const MENU_QUIT: &str = "Sair";
/// O título da confirmação do "Sair".
pub const QUIT_TITLE: &str = "Sair do Scriba?";
/// A pergunta no fim da confirmação.
pub const QUIT_QUESTION: &str = "Sair mesmo assim?";
/// O motivo que o Windows mostra enquanto o Scriba segura o desligamento.
pub const END_SESSION_REASON: &str = "Finalizando a gravação…";

/// O que a janela da bandeja recebeu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Clique no ícone ou "Abrir o Scriba".
    Open,
    /// "Sair" no menu.
    Quit,
    /// Logoff ou desligamento (`WM_ENDSESSION`).
    EndSession,
}

/// O que a área de trabalho faz pela bandeja.
pub trait Desk {
    /// Abre a UI no navegador.
    fn open(&self);
    /// Pergunta se sai mesmo assim; `true` = sair.
    fn confirm(&self, title: &str, text: &str) -> bool;
}

/// O que se perde ao sair agora.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pending {
    pub recording: bool,
    /// Os envios na fila ou transcrevendo (as gravações voltam na próxima abertura).
    pub transcribing: Vec<String>,
    /// Algum "Guardar o áudio?" sem resposta.
    pub unanswered_audio: bool,
}

impl Pending {
    pub fn of(status: &Status, jobs: &[Job]) -> Self {
        let mut pending = Self {
            recording: matches!(status, Status::Recording { .. }),
            ..Self::default()
        };
        for job in jobs {
            match &job.state {
                State::Queued | State::Running { .. } if job.media != Media::Recording => {
                    pending.transcribing.push(job.name.clone());
                }
                State::Done { audio, .. } if !audio.answered() => {
                    pending.unanswered_audio = true;
                }
                _ => {}
            }
        }
        pending
    }
}

/// O texto da confirmação do "Sair"; `None` quando nada se perde e o app sai direto.
pub fn quit_warning(pending: &Pending) -> Option<String> {
    let mut lines = Vec::new();
    if pending.recording {
        lines.push(
            "A gravação em andamento para agora. O que foi gravado será transcrito quando o \
             Scriba abrir de novo."
                .to_string(),
        );
    }
    match pending.transcribing.as_slice() {
        [] => {}
        [name] => lines.push(format!("A transcrição de {name} será cancelada.")),
        names => lines.push(format!(
            "As {} transcrições na fila serão canceladas.",
            names.len()
        )),
    }
    if pending.unanswered_audio {
        lines.push("O áudio sem resposta será descartado.".into());
    }
    (!lines.is_empty()).then(|| lines.join("\n\n"))
}

/// A dica do ícone agora.
pub fn tooltip(recording: bool) -> &'static str {
    if recording {
        TOOLTIP_RECORDING
    } else {
        TOOLTIP
    }
}

/// Os dois ícones da bandeja (recursos do exe).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayIcon {
    /// Glifo escuro, para a barra clara.
    Dark,
    /// Glifo claro, para a barra escura.
    Light,
}

/// O ícone pelo `SystemUsesLightTheme` do registro: a barra de tarefas segue o tema do
/// sistema, que é escuro por padrão (e quando o valor falta, como no Windows 10 antigo).
pub fn icon_for(system_uses_light_theme: Option<u32>) -> TrayIcon {
    match system_uses_light_theme {
        Some(v) if v != 0 => TrayIcon::Dark,
        _ => TrayIcon::Light,
    }
}

/// Trata os eventos até o canal fechar. `pending` diz o que se perde ao sair agora.
pub fn run(
    events: Receiver<TrayEvent>,
    desk: &dyn Desk,
    pending: &dyn Fn() -> Pending,
    shutdown: &Shutdown,
) {
    for event in events {
        match event {
            TrayEvent::Open => desk.open(),
            TrayEvent::Quit => {
                let go = match quit_warning(&pending()) {
                    None => true,
                    Some(text) => desk.confirm(QUIT_TITLE, &text),
                };
                if go {
                    shutdown.request(Reason::Tray);
                }
            }
            TrayEvent::EndSession => shutdown.request(Reason::EndSession),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::sync::mpsc;

    use super::*;
    use crate::clock::LocalTime;
    use crate::jobs::{Audio, JobId, Outputs, Stage};

    #[derive(Default)]
    struct FakeDesk {
        opened: Cell<usize>,
        asked: RefCell<Vec<String>>,
        answer: bool,
    }

    impl Desk for FakeDesk {
        fn open(&self) {
            self.opened.set(self.opened.get() + 1);
        }
        fn confirm(&self, title: &str, text: &str) -> bool {
            assert_eq!(title, QUIT_TITLE);
            self.asked.borrow_mut().push(text.into());
            self.answer
        }
    }

    fn drive(events: &[TrayEvent], desk: &FakeDesk, pending: Pending) -> Shutdown {
        let (tx, rx) = mpsc::channel();
        for e in events {
            tx.send(*e).unwrap();
        }
        drop(tx);
        let shutdown = Shutdown::new();
        run(rx, desk, &|| pending.clone(), &shutdown);
        shutdown
    }

    #[test]
    fn abrir_abre_o_navegador_e_nao_fecha() {
        let desk = FakeDesk::default();
        let shutdown = drive(
            &[TrayEvent::Open, TrayEvent::Open],
            &desk,
            Pending::default(),
        );
        assert_eq!(desk.opened.get(), 2);
        assert_eq!(shutdown.reason(), None);
    }

    #[test]
    fn sair_sem_nada_pendente_sai_direto() {
        let desk = FakeDesk::default();
        let shutdown = drive(&[TrayEvent::Quit], &desk, Pending::default());
        assert!(desk.asked.borrow().is_empty());
        assert_eq!(shutdown.reason(), Some(Reason::Tray));
    }

    #[test]
    fn sair_gravando_pergunta_e_respeita_a_resposta() {
        let pending = Pending {
            recording: true,
            ..Pending::default()
        };
        let no = FakeDesk::default();
        let shutdown = drive(&[TrayEvent::Quit], &no, pending.clone());
        assert_eq!(no.asked.borrow().len(), 1);
        assert!(no.asked.borrow()[0].contains("gravação"));
        assert_eq!(shutdown.reason(), None);

        let yes = FakeDesk {
            answer: true,
            ..FakeDesk::default()
        };
        let shutdown = drive(&[TrayEvent::Quit], &yes, pending);
        assert_eq!(shutdown.reason(), Some(Reason::Tray));
    }

    #[test]
    fn fim_da_sessao_fecha_sem_perguntar() {
        let desk = FakeDesk::default();
        let pending = Pending {
            recording: true,
            ..Pending::default()
        };
        let shutdown = drive(&[TrayEvent::EndSession], &desk, pending);
        assert!(desk.asked.borrow().is_empty());
        assert_eq!(shutdown.reason(), Some(Reason::EndSession));
    }

    #[test]
    fn aviso_de_cada_combinacao() {
        assert_eq!(quit_warning(&Pending::default()), None);
        let rec = "A gravação em andamento para agora. O que foi gravado será transcrito \
                   quando o Scriba abrir de novo.";
        let one = "A transcrição de reuniao.mp4 será cancelada.";
        let audio = "O áudio sem resposta será descartado.";
        for recording in [false, true] {
            for names in [vec![], vec!["reuniao.mp4".to_string()]] {
                for unanswered_audio in [false, true] {
                    let pending = Pending {
                        recording,
                        transcribing: names.clone(),
                        unanswered_audio,
                    };
                    let mut expected = Vec::new();
                    if recording {
                        expected.push(rec);
                    }
                    if !names.is_empty() {
                        expected.push(one);
                    }
                    if unanswered_audio {
                        expected.push(audio);
                    }
                    let want = (!expected.is_empty()).then(|| expected.join("\n\n"));
                    assert_eq!(quit_warning(&pending), want, "{pending:?}");
                }
            }
        }
        let many = Pending {
            transcribing: vec!["a.mp3".into(), "b.mp3".into(), "c.mp3".into()],
            ..Pending::default()
        };
        assert_eq!(
            quit_warning(&many).unwrap(),
            "As 3 transcrições na fila serão canceladas."
        );
    }

    fn job(name: &str, media: Media, state: State) -> Job {
        Job {
            id: JobId::generate(),
            name: name.into(),
            media,
            size: 1,
            duration_ms: 0,
            created: LocalTime::now(),
            estimate: None,
            input: PathBuf::from("x"),
            state,
        }
    }

    #[test]
    fn pendencias_vem_da_fila_e_da_gravacao() {
        let done = |audio| State::Done {
            outputs: Outputs::default(),
            audio,
        };
        let jobs = vec![
            job("fila.mp3", Media::Audio, State::Queued),
            job(
                "rodando.mp4",
                Media::Video,
                State::Running {
                    stage: Stage::Transcribing,
                    pct: 10,
                    canceling: false,
                },
            ),
            // A gravação na fila volta na próxima abertura: não se perde.
            job("Gravação de 20/10, 14:30", Media::Recording, State::Queued),
            job("pronto.mp3", Media::Audio, done(Audio::Discarded)),
            job(
                "falhou.mp3",
                Media::Audio,
                State::Failed {
                    message: "x".into(),
                },
            ),
        ];
        let idle = Status::Idle { last: None };
        let pending = Pending::of(&idle, &jobs);
        assert_eq!(pending.transcribing, ["fila.mp3", "rodando.mp4"]);
        assert!(!pending.recording && !pending.unanswered_audio);

        let unanswered = [job("pronto.mp3", Media::Audio, done(Audio::PENDING))];
        assert!(Pending::of(&idle, &unanswered).unanswered_audio);
    }

    #[test]
    fn icone_e_dica() {
        assert_eq!(icon_for(Some(1)), TrayIcon::Dark);
        assert_eq!(icon_for(Some(0)), TrayIcon::Light);
        assert_eq!(icon_for(None), TrayIcon::Light);
        assert_eq!(tooltip(false), "Scriba");
        assert_eq!(tooltip(true), "Scriba — gravando");
    }
}
