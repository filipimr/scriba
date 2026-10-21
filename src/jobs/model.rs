//! Modelo do job e a máquina de estados, como funções puras: o store aplica os eventos e
//! ignora as transições inválidas.

use std::path::PathBuf;

use super::id::JobId;
use crate::capture::Recording;
use crate::clock::LocalTime;

/// Formatos aceitos no envio: o primeiro filtro, pela extensão, antes do probe.
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
    /// Uma gravação ao vivo: as duas trilhas WAV numa pasta.
    Recording,
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

/// As etapas do pipeline, em ordem: todo job extrai o áudio e transcreve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    ExtractingAudio,
    Transcribing,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::ExtractingAudio => "Extraindo áudio",
            Self::Transcribing => "Transcrevendo",
        }
    }
}

/// O que o probe do envio descobriu sobre o arquivo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Probe {
    /// Pelo conteúdo: um MP4 só com áudio é áudio.
    pub media: Media,
    /// 0 quando o arquivo não informa.
    pub duration_ms: u64,
    pub estimate: Option<Estimate>,
}

/// Quanto a transcrição deve levar, e onde.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Estimate {
    pub secs: u64,
    pub gpu: bool,
}

/// A pasta da saída e os arquivos da transcrição dentro dela.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub dir: PathBuf,
    pub txt: PathBuf,
    pub md: PathBuf,
}

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
        /// A resposta à pergunta "Guardar o áudio?".
        audio: Audio,
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
    /// "Tentar de novo" num job que falhou: volta para a fila.
    Retry,
    /// "Guardar o áudio?" num job concluído.
    Audio(AudioEvent),
}

/// "Guardar o áudio?" de um job concluído. O envio fica em `uploads/` até a resposta:
/// "Descartar" o apaga; "Guardar áudio" abre o "Salvar como", grava o `.m4a` e o apaga.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Audio {
    /// Esperando a resposta; `error` é o motivo de a última tentativa de guardar ter
    /// falhado.
    Pending {
        error: Option<String>,
    },
    /// A janela "Salvar como" está aberta.
    Choosing,
    Saving {
        pct: u8,
        /// O usuário pediu para cancelar; a gravação ainda não parou.
        canceling: bool,
    },
    Saved {
        path: PathBuf,
    },
    Discarded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioEvent {
    Discard,
    /// "Guardar áudio": a janela abre.
    Choose,
    /// A janela fechou: o arquivo escolhido, ou `None` se o usuário cancelou.
    Chosen(Option<PathBuf>),
    Progress(u8),
    Cancel,
    /// O fim da gravação, com o arquivo gravado; um erro antes da escolha é a janela que
    /// não abriu.
    Finished(Result<PathBuf, Failure>),
}

impl Audio {
    /// A pergunta, sem resposta ainda.
    pub const PENDING: Self = Self::Pending { error: None };

    pub fn apply(&self, event: AudioEvent) -> Result<Self, Invalid> {
        use Audio::{Choosing, Discarded, Pending, Saved, Saving};
        match (self, event) {
            (Pending { .. }, AudioEvent::Discard) => Ok(Discarded),
            (Pending { .. }, AudioEvent::Choose) => Ok(Choosing),
            (Choosing, AudioEvent::Chosen(None)) => Ok(Self::PENDING),
            (Choosing, AudioEvent::Chosen(Some(_))) => Ok(Saving {
                pct: 0,
                canceling: false,
            }),
            (Choosing, AudioEvent::Finished(Err(Failure::Error(message)))) => Ok(Pending {
                error: Some(message),
            }),
            (Saving { canceling, .. }, AudioEvent::Progress(pct)) => Ok(Saving {
                pct: pct.min(100),
                canceling: *canceling,
            }),
            (Saving { pct, .. }, AudioEvent::Cancel) => Ok(Saving {
                pct: *pct,
                canceling: true,
            }),
            // O arquivo ficou pronto: vale mesmo com um cancelamento pedido no fim.
            (Saving { .. }, AudioEvent::Finished(Ok(path))) => Ok(Saved { path }),
            // Cancelado, ou um erro depois do pedido: a pergunta volta, sem mensagem.
            (
                Saving {
                    canceling: true, ..
                },
                AudioEvent::Finished(Err(_)),
            )
            | (Saving { .. }, AudioEvent::Finished(Err(Failure::Canceled))) => Ok(Self::PENDING),
            (Saving { .. }, AudioEvent::Finished(Err(Failure::Error(message)))) => Ok(Pending {
                error: Some(message),
            }),
            // Cancelar o que não está gravando não muda nada.
            (_, AudioEvent::Cancel) => Ok(self.clone()),
            _ => Err(Invalid),
        }
    }

    /// A janela está aberta ou o `.m4a` está sendo gravado: o envio ainda é usado.
    pub fn busy(&self) -> bool {
        matches!(self, Self::Choosing | Self::Saving { .. })
    }

    /// Respondida: o envio já foi apagado.
    pub fn answered(&self) -> bool {
        matches!(self, Self::Saved { .. } | Self::Discarded)
    }
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
            (Running { .. }, Event::Finished(Ok(outputs))) => Ok(Done {
                outputs,
                audio: Audio::PENDING,
            }),
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
            (Failed { .. }, Event::Retry) => Ok(Queued),
            (Done { outputs, audio }, Event::Audio(event)) => Ok(Done {
                outputs: outputs.clone(),
                audio: audio.apply(event)?,
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

/// Um arquivo enviado ou uma gravação.
#[derive(Clone, Debug)]
pub struct Job {
    pub id: JobId,
    /// Nome original, só para exibir: nunca vira caminho. Numa gravação, "Gravação de
    /// 20/10, 14:30".
    pub name: String,
    pub media: Media,
    pub size: u64,
    /// Do probe; 0 quando o arquivo não informa.
    pub duration_ms: u64,
    /// Hora local do envio (ou do Gravar), a mesma do nome da pasta da saída.
    pub created: LocalTime,
    pub estimate: Option<Estimate>,
    /// Cópia enviada, em `uploads/`; numa gravação, a pasta dela em `gravacoes/`. Num job
    /// concluído, fica até a resposta a "Guardar o áudio?"; num que falhou, para o "Tentar
    /// de novo". Apagada quando o job é cancelado ou removido, e ao fechar o app (a
    /// gravação, só depois de transcrita: senão ela volta na próxima abertura).
    pub input: PathBuf,
    pub state: State,
}

/// Quanto a gravação custa, no máximo, sobre um arquivo da mesma duração: medido na etapa 6.4
/// (`custo_da_gravacao`), até 2,06× sem fone; 1,1–1,35× com fone.
pub const RECORDING_COST: f64 = 2.2;

/// O começo do nome de uma gravação na lista.
pub const RECORDING_PREFIX: &str = "Gravação de";

impl Job {
    /// O job de uma gravação terminada. `file_estimate` é a estimativa de um arquivo da
    /// mesma duração: as duas trilhas custam até [`RECORDING_COST`] vezes isso (sem fone, o
    /// eco também é transcrito antes de sair).
    pub fn recording(rec: Recording, file_estimate: Option<Estimate>) -> Self {
        let c = rec.created;
        Self {
            id: rec.id,
            name: format!(
                "{RECORDING_PREFIX} {:02}/{:02}, {:02}:{:02}",
                c.day, c.month, c.hour, c.minute
            ),
            media: Media::Recording,
            size: rec.size,
            duration_ms: rec.duration_ms,
            created: rec.created,
            estimate: file_estimate.map(|e| Estimate {
                secs: (e.secs as f64 * RECORDING_COST).ceil() as u64,
                ..e
            }),
            input: rec.dir,
            state: State::Queued,
        }
    }
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

    fn done() -> State {
        State::Done {
            outputs: Outputs::default(),
            audio: Audio::PENDING,
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
                Event::Finished(Ok(Outputs::default())),
                done(),
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
            (done(), Event::Cancel, done()),
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
            .apply(Event::Progress(Stage::ExtractingAudio, 250))
            .unwrap();
        assert_eq!(
            s,
            State::Running {
                stage: Stage::ExtractingAudio,
                pct: 100,
                canceling: false
            }
        );
    }

    #[test]
    fn tentar_de_novo_so_vale_em_falhou() {
        for state in [State::Queued, running(false), done(), State::Canceled] {
            assert_eq!(state.apply(Event::Retry), Err(Invalid), "{state:?}");
        }
        let again = State::Failed {
            message: "x".into(),
        }
        .apply(Event::Retry)
        .unwrap();
        assert_eq!(again, State::Queued);
        assert!(again.apply(Event::Start(Stage::ExtractingAudio)).is_ok());
    }

    #[test]
    fn job_de_gravacao() {
        let created = LocalTime {
            year: 2026,
            month: 10,
            day: 20,
            hour: 9,
            minute: 5,
            second: 59,
            offset_minutes: -180,
        };
        let rec = Recording {
            id: JobId::generate(),
            dir: "C:/dados/gravacoes/x".into(),
            created,
            duration_ms: 90_000,
            size: 11_520_088,
            mic: "Microfone".into(),
            output: "Alto-falantes".into(),
            end: crate::capture::End::User,
            stats: Default::default(),
        };
        let estimate = Estimate {
            secs: 40,
            gpu: true,
        };
        let job = Job::recording(rec.clone(), Some(estimate));
        assert_eq!(job.id, rec.id);
        assert_eq!(job.name, "Gravação de 20/10, 09:05");
        assert_eq!(job.media, Media::Recording);
        assert_eq!((job.size, job.duration_ms), (11_520_088, 90_000));
        assert_eq!(job.created, created);
        assert_eq!(job.input, rec.dir);
        assert_eq!(job.state, State::Queued);
        assert_eq!(
            job.estimate,
            Some(Estimate {
                secs: 88,
                gpu: true
            })
        );
        assert_eq!(Job::recording(rec, None).estimate, None);
    }

    #[test]
    fn formato_pela_extensao() {
        assert_eq!(Media::detect("Reunião.MP4"), Some((Media::Video, "mp4")));
        assert_eq!(Media::detect("a.tar.ogg"), Some((Media::Audio, "ogg")));
        assert_eq!(Media::detect("nota.pdf"), None);
        assert_eq!(Media::detect("sem-extensao"), None);
        assert_eq!(Media::detect(".wav"), Some((Media::Audio, "wav")));
    }

    fn saving(canceling: bool) -> Audio {
        Audio::Saving { pct: 40, canceling }
    }

    #[test]
    fn guardar_o_audio() {
        let path = || PathBuf::from("C:/Áudios/reunião.m4a");
        let error = || Failure::Error("Disco cheio.".into());
        let failed = || Audio::Pending {
            error: Some("Disco cheio.".into()),
        };
        let cases = [
            (Audio::PENDING, AudioEvent::Discard, Audio::Discarded),
            (Audio::PENDING, AudioEvent::Choose, Audio::Choosing),
            (failed(), AudioEvent::Choose, Audio::Choosing),
            (Audio::Choosing, AudioEvent::Chosen(None), Audio::PENDING),
            (
                Audio::Choosing,
                AudioEvent::Chosen(Some(path())),
                Audio::Saving {
                    pct: 0,
                    canceling: false,
                },
            ),
            (
                Audio::Choosing,
                AudioEvent::Finished(Err(error())),
                failed(),
            ),
            (
                saving(false),
                AudioEvent::Progress(250),
                Audio::Saving {
                    pct: 100,
                    canceling: false,
                },
            ),
            (saving(false), AudioEvent::Cancel, saving(true)),
            (
                saving(true),
                AudioEvent::Progress(50),
                Audio::Saving {
                    pct: 50,
                    canceling: true,
                },
            ),
            (
                saving(true),
                AudioEvent::Finished(Ok(path())),
                Audio::Saved { path: path() },
            ),
            (
                saving(true),
                AudioEvent::Finished(Err(error())),
                Audio::PENDING,
            ),
            (
                saving(false),
                AudioEvent::Finished(Err(Failure::Canceled)),
                Audio::PENDING,
            ),
            (saving(false), AudioEvent::Finished(Err(error())), failed()),
            (Audio::Discarded, AudioEvent::Cancel, Audio::Discarded),
            (Audio::PENDING, AudioEvent::Cancel, Audio::PENDING),
        ];
        for (audio, event, expected) in cases {
            assert_eq!(
                audio.apply(event.clone()),
                Ok(expected),
                "{audio:?} + {event:?}"
            );
        }
        for (audio, event) in [
            (Audio::Discarded, AudioEvent::Choose),
            (Audio::Saved { path: path() }, AudioEvent::Discard),
            (Audio::Choosing, AudioEvent::Discard),
            (saving(false), AudioEvent::Choose),
            (saving(false), AudioEvent::Discard),
            (Audio::PENDING, AudioEvent::Chosen(None)),
            (Audio::PENDING, AudioEvent::Finished(Ok(path()))),
        ] {
            assert_eq!(
                audio.apply(event.clone()),
                Err(Invalid),
                "{audio:?} + {event:?}"
            );
        }
        assert!(Audio::Choosing.busy() && saving(true).busy() && !Audio::PENDING.busy());
        assert!(Audio::Discarded.answered() && !failed().answered());
    }

    #[test]
    fn so_o_job_concluido_responde_a_pergunta() {
        for state in [
            State::Queued,
            running(false),
            State::Canceled,
            State::Failed {
                message: "x".into(),
            },
        ] {
            assert_eq!(
                state.apply(Event::Audio(AudioEvent::Discard)),
                Err(Invalid),
                "{state:?}"
            );
        }
        let after = done().apply(Event::Audio(AudioEvent::Discard)).unwrap();
        assert_eq!(
            after,
            State::Done {
                outputs: Outputs::default(),
                audio: Audio::Discarded
            }
        );
        assert_eq!(after.apply(Event::Audio(AudioEvent::Discard)), Err(Invalid));
    }

    fn audio_event() -> impl Strategy<Value = AudioEvent> {
        prop_oneof![
            Just(AudioEvent::Discard),
            Just(AudioEvent::Choose),
            Just(AudioEvent::Chosen(None)),
            Just(AudioEvent::Chosen(Some("a.m4a".into()))),
            any::<u8>().prop_map(AudioEvent::Progress),
            Just(AudioEvent::Cancel),
            Just(AudioEvent::Finished(Ok("a.m4a".into()))),
            Just(AudioEvent::Finished(Err(Failure::Canceled))),
            "[a-z]{1,8}".prop_map(|m| AudioEvent::Finished(Err(Failure::Error(m)))),
        ]
    }

    proptest! {
        /// Como o store: aplica cada evento e ignora os inválidos.
        #[test]
        fn pergunta_do_audio_mantem_as_invariantes(
            events in prop::collection::vec(audio_event(), 0..40)
        ) {
            let mut audio = Audio::PENDING;
            for event in events {
                let before = audio.clone();
                if let Ok(next) = before.apply(event.clone()) {
                    audio = next;
                }
                // A resposta é definitiva.
                if before.answered() {
                    prop_assert_eq!(&audio, &before);
                }
                // Só se guarda o que passou pela janela e pela gravação.
                if matches!(audio, Audio::Saved { .. }) && !before.answered() {
                    let was_saving = matches!(before, Audio::Saving { .. });
                    prop_assert!(was_saving, "{:?}", before);
                }
                if let Audio::Saving { pct, canceling } = &audio {
                    prop_assert!(*pct <= 100);
                    // Um pedido de cancelamento nunca se perde enquanto grava.
                    if matches!(before, Audio::Saving { canceling: true, .. }) {
                        prop_assert!(*canceling);
                    }
                }
            }
        }
    }

    fn stage() -> impl Strategy<Value = Stage> {
        prop_oneof![Just(Stage::ExtractingAudio), Just(Stage::Transcribing)]
    }

    fn event() -> impl Strategy<Value = Event> {
        prop_oneof![
            stage().prop_map(Event::Start),
            (stage(), any::<u8>()).prop_map(|(s, p)| Event::Progress(s, p)),
            Just(Event::Cancel),
            Just(Event::Finished(Ok(Outputs::default()))),
            Just(Event::Finished(Err(Failure::Canceled))),
            "[a-z]{1,8}".prop_map(|m| Event::Finished(Err(Failure::Error(m)))),
            Just(Event::Panicked),
            Just(Event::Retry),
            audio_event().prop_map(Event::Audio),
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
                // Estado final é absorvente, exceto "Tentar de novo" num job que falhou, que
                // volta para a fila, e a resposta ao "Guardar o áudio?", que mantém o job
                // concluído com as mesmas saídas.
                let retried = matches!(before, State::Failed { .. }) && event == Event::Retry;
                if retried {
                    prop_assert_eq!(&state, &State::Queued);
                } else if let State::Done { outputs, .. } = &before {
                    let State::Done { outputs: after, .. } = &state else {
                        panic!("{state:?}");
                    };
                    prop_assert_eq!(outputs, after);
                } else if before.is_terminal() || event == Event::Retry {
                    prop_assert_eq!(&state, &before);
                }
                // Só o job concluído responde à pergunta.
                if matches!(event, Event::Audio(_)) && !matches!(before, State::Done { .. }) {
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
