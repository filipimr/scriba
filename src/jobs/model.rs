//! Modelo do job e a máquina de estados, como funções puras: o store aplica os eventos e
//! ignora as transições inválidas.

use std::path::PathBuf;

use super::id::JobId;

/// Formatos aceitos no envio, conferidos pela extensão até o probe do FFmpeg (Fase 4).
pub const FORMATS: &[(&str, Media)] = &[
    ("mp4", Media::Video),
    ("mov", Media::Video),
    ("mkv", Media::Video),
    ("mp3", Media::Audio),
    ("m4a", Media::Audio),
    ("wav", Media::Audio),
    ("ogg", Media::Audio),
];

/// Mensagem do job cujo pipeline entrou em pânico: o detalhe vai para o stderr, não para a
/// tela.
pub const PANIC_MESSAGE: &str = "Erro inesperado ao processar o arquivo.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Video,
    Audio,
}

impl Media {
    /// Tipo e extensão normalizada, pelo nome do arquivo; `None` se o formato não é aceito.
    pub fn detect(name: &str) -> Option<(Self, &'static str)> {
        let (_, ext) = name.rsplit_once('.')?;
        FORMATS
            .iter()
            .find(|(e, _)| e.eq_ignore_ascii_case(ext))
            .map(|(e, media)| (*media, *e))
    }
}

/// O que fazer com o arquivo: transcrever, compactar ou os dois.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    transcribe: bool,
    compress: bool,
}

impl Options {
    /// `None` sem nenhuma das duas: todo job tem pelo menos uma etapa.
    pub fn new(transcribe: bool, compress: bool) -> Option<Self> {
        (transcribe || compress).then_some(Self {
            transcribe,
            compress,
        })
    }

    /// Etapas do pipeline, em ordem.
    pub fn stages(self) -> Vec<Stage> {
        let mut stages = Vec::new();
        if self.transcribe {
            stages.extend([Stage::ExtractingAudio, Stage::Transcribing]);
        }
        if self.compress {
            stages.push(Stage::Compressing);
        }
        stages
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    ExtractingAudio,
    Transcribing,
    Compressing,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::ExtractingAudio => "Extraindo áudio",
            Self::Transcribing => "Transcrevendo",
            Self::Compressing => "Compactando",
        }
    }
}

/// Arquivos gerados pelo job (vazio até a Fase 4).
pub type Outputs = Vec<PathBuf>;

/// Por que o pipeline não terminou.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Canceled,
    /// Mensagem para o usuário: o que aconteceu e o que fazer.
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Queued,
    Running {
        stage: Stage,
        pct: u8,
        /// O usuário pediu para cancelar; o pipeline ainda não parou.
        canceling: bool,
    },
    Done {
        outputs: Outputs,
    },
    Failed {
        message: String,
    },
    Canceled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// O worker tirou o job da fila.
    Start(Stage),
    Progress(Stage, u8),
    /// Pedido do usuário.
    Cancel,
    Finished(Result<Outputs, Failure>),
    Panicked,
}

/// Evento que não vale no estado atual (ex.: começar um job cancelado).
#[derive(Debug, PartialEq, Eq)]
pub struct Invalid;

impl State {
    pub fn apply(&self, event: Event) -> Result<Self, Invalid> {
        use State::{Canceled, Done, Failed, Queued, Running};
        match (self, event) {
            (Queued, Event::Start(stage)) => Ok(Running {
                stage,
                pct: 0,
                canceling: false,
            }),
            (Queued, Event::Cancel) => Ok(Canceled),
            (Running { canceling, .. }, Event::Progress(stage, pct)) => Ok(Running {
                stage,
                pct: pct.min(100),
                canceling: *canceling,
            }),
            (Running { stage, pct, .. }, Event::Cancel) => Ok(Running {
                stage: *stage,
                pct: *pct,
                canceling: true,
            }),
            // O trabalho terminou: o resultado vale mesmo com um cancelamento pedido no fim.
            (Running { .. }, Event::Finished(Ok(outputs))) => Ok(Done { outputs }),
            // Um erro depois do pedido é consequência da interrupção.
            (
                Running {
                    canceling: true, ..
                },
                Event::Finished(Err(_)),
            )
            | (Running { .. }, Event::Finished(Err(Failure::Canceled))) => Ok(Canceled),
            (Running { .. }, Event::Finished(Err(Failure::Error(message)))) => {
                Ok(Failed { message })
            }
            (Running { .. }, Event::Panicked) => Ok(Failed {
                message: PANIC_MESSAGE.into(),
            }),
            // Cancelar o que já terminou não muda nada.
            (Done { .. } | Failed { .. } | Canceled, Event::Cancel) => Ok(self.clone()),
            _ => Err(Invalid),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done { .. } | Self::Failed { .. } | Self::Canceled
        )
    }
}

/// Um arquivo enviado e o que fazer com ele.
#[derive(Clone, Debug)]
pub struct Job {
    pub id: JobId,
    /// Nome original, só para exibir: nunca vira caminho.
    pub name: String,
    pub media: Media,
    pub size: u64,
    pub options: Options,
    /// Cópia enviada, em `uploads/`; apagada quando o job termina ou é removido.
    pub input: PathBuf,
    pub state: State,
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn running(canceling: bool) -> State {
        State::Running {
            stage: Stage::Transcribing,
            pct: 40,
            canceling,
        }
    }

    #[test]
    fn cancelar_na_fila_encerra_e_impede_o_inicio() {
        let canceled = State::Queued.apply(Event::Cancel).unwrap();
        assert_eq!(canceled, State::Canceled);
        assert_eq!(
            canceled.apply(Event::Start(Stage::Transcribing)),
            Err(Invalid)
        );
    }

    #[test]
    fn semantica_do_cancelamento() {
        let error = || Failure::Error("Algo deu errado.".into());
        let cases = [
            // Pedido no meio: o pipeline ainda roda até ver a flag.
            (running(false), Event::Cancel, running(true)),
            // Terminou antes de ver a flag: o resultado vale.
            (
                running(true),
                Event::Finished(Ok(vec![])),
                State::Done { outputs: vec![] },
            ),
            (
                running(true),
                Event::Finished(Err(error())),
                State::Canceled,
            ),
            (
                running(false),
                Event::Finished(Err(error())),
                State::Failed {
                    message: "Algo deu errado.".into(),
                },
            ),
            (
                running(false),
                Event::Finished(Err(Failure::Canceled)),
                State::Canceled,
            ),
            // Panic é bug, mesmo durante o cancelamento.
            (
                running(true),
                Event::Panicked,
                State::Failed {
                    message: PANIC_MESSAGE.into(),
                },
            ),
            (
                State::Done { outputs: vec![] },
                Event::Cancel,
                State::Done { outputs: vec![] },
            ),
            (State::Canceled, Event::Cancel, State::Canceled),
        ];
        for (state, event, expected) in cases {
            assert_eq!(
                state.apply(event.clone()),
                Ok(expected),
                "{state:?} + {event:?}"
            );
        }
    }

    #[test]
    fn progresso_limita_a_100() {
        let s = running(false)
            .apply(Event::Progress(Stage::Compressing, 250))
            .unwrap();
        assert_eq!(
            s,
            State::Running {
                stage: Stage::Compressing,
                pct: 100,
                canceling: false
            }
        );
    }

    #[test]
    fn opcoes_definem_as_etapas() {
        use Stage::{Compressing, ExtractingAudio, Transcribing};
        assert_eq!(Options::new(false, false), None);
        let stages = |t, c| Options::new(t, c).unwrap().stages();
        assert_eq!(stages(true, false), [ExtractingAudio, Transcribing]);
        assert_eq!(stages(false, true), [Compressing]);
        assert_eq!(
            stages(true, true),
            [ExtractingAudio, Transcribing, Compressing]
        );
    }

    #[test]
    fn formato_pela_extensao() {
        assert_eq!(Media::detect("Reunião.MP4"), Some((Media::Video, "mp4")));
        assert_eq!(Media::detect("a.tar.ogg"), Some((Media::Audio, "ogg")));
        assert_eq!(Media::detect("nota.pdf"), None);
        assert_eq!(Media::detect("sem-extensao"), None);
        assert_eq!(Media::detect(".wav"), Some((Media::Audio, "wav")));
    }

    fn stage() -> impl Strategy<Value = Stage> {
        prop_oneof![
            Just(Stage::ExtractingAudio),
            Just(Stage::Transcribing),
            Just(Stage::Compressing),
        ]
    }

    fn event() -> impl Strategy<Value = Event> {
        prop_oneof![
            stage().prop_map(Event::Start),
            (stage(), any::<u8>()).prop_map(|(s, p)| Event::Progress(s, p)),
            Just(Event::Cancel),
            Just(Event::Finished(Ok(Vec::new()))),
            Just(Event::Finished(Err(Failure::Canceled))),
            "[a-z]{1,8}".prop_map(|m| Event::Finished(Err(Failure::Error(m)))),
            Just(Event::Panicked),
        ]
    }

    proptest! {
        /// Como o store: aplica cada evento e ignora os inválidos.
        #[test]
        fn maquina_de_estados_mantem_as_invariantes(
            events in prop::collection::vec(event(), 0..40)
        ) {
            let mut state = State::Queued;
            let mut canceled_in_queue = false;
            for event in events {
                let before = state.clone();
                if let Ok(next) = before.apply(event.clone()) {
                    state = next;
                }
                // Estado final é absorvente.
                if before.is_terminal() {
                    prop_assert_eq!(&state, &before);
                }
                // Só começa quem estava na fila.
                if matches!(event, Event::Start(_)) && before != State::Queued {
                    prop_assert_eq!(&state, &before);
                }
                if let State::Running { pct, canceling, .. } = &state {
                    prop_assert!(*pct <= 100);
                    // Um pedido de cancelamento nunca se perde enquanto o job roda.
                    if matches!(before, State::Running { canceling: true, .. }) {
                        prop_assert!(*canceling);
                    }
                }
                if before == State::Queued && event == Event::Cancel {
                    canceled_in_queue = true;
                }
                // Cancelado na fila nunca roda.
                if canceled_in_queue {
                    prop_assert_eq!(&state, &State::Canceled);
                }
            }
        }
    }
}
