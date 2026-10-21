//! A fonte do áudio: o trait [`AudioSource`] (o cpal no app, o falso nos testes), os
//! dispositivos e os ring buffers SPSC por onde os callbacks entregam os pacotes à thread de
//! escrita. Os callbacks só copiam: nada de alocar, travar ou esperar.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

/// As duas trilhas da gravação.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Track {
    /// O microfone: "Você".
    Mic,
    /// O som do PC (loopback da saída): "Outros".
    System,
}

impl Track {
    pub const ALL: [Self; 2] = [Self::Mic, Self::System];

    pub fn index(self) -> usize {
        match self {
            Self::Mic => 0,
            Self::System => 1,
        }
    }

    /// O WAV da trilha, dentro da pasta da gravação.
    pub fn file(self) -> &'static str {
        match self {
            Self::Mic => "mic.wav",
            Self::System => "sistema.wav",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// Estável entre execuções; vai para o `config.json`.
    pub id: String,
    pub name: String,
    /// O padrão do Windows agora.
    pub default: bool,
    pub bluetooth: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    pub inputs: Vec<Device>,
    pub outputs: Vec<Device>,
}

/// Os dispositivos escolhidos nos Ajustes; `None` é o padrão do Windows, que acompanha as
/// trocas durante a gravação.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    pub mic: Option<String>,
    pub output: Option<String>,
}

impl Choice {
    pub fn device(&self, track: Track) -> Option<&str> {
        match track {
            Track::Mic => self.mic.as_deref(),
            Track::System => self.output.as_deref(),
        }
    }
}

/// O formato que o dispositivo entrega (f32 intercalado).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u16,
}

/// O cabeçalho de um pacote no ring: o instante de captura do 1º frame, no relógio de
/// [`AudioSource::now_ns`], e quantos frames vêm atrás.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet {
    pub t_ns: u64,
    pub frames: u32,
}

/// O que um stream avisa pelo callback de erro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// O stream precisa ser reaberto (o padrão do Windows trocou, ou o formato mudou).
    Invalidated,
    /// O dispositivo sumiu, sem substituto.
    Lost,
    /// Um pacote se perdeu no driver; o alinhamento cobre o buraco.
    Xrun,
}

/// Um evento de um stream. `generation` separa os avisos de um stream já fechado dos do
/// stream reaberto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub track: Track,
    pub generation: u64,
    pub kind: EventKind,
}

/// Por onde os streams avisam.
#[derive(Clone)]
pub struct Events {
    pub track: Track,
    pub generation: u64,
    tx: Sender<Event>,
}

impl Events {
    pub fn new(track: Track, generation: u64, tx: Sender<Event>) -> Self {
        Self {
            track,
            generation,
            tx,
        }
    }

    pub fn send(&self, kind: EventKind) {
        let _ = self.tx.send(Event {
            track: self.track,
            generation: self.generation,
            kind,
        });
    }
}

/// Uma trilha aberta: o lado de leitura do ring, o nome do dispositivo e os streams (fechados
/// quando o valor sai de cena).
pub struct Opened {
    pub consumer: Consumer,
    pub name: String,
    pub streams: Box<dyn Send>,
}

pub trait AudioSource: Send + Sync {
    fn devices(&self) -> Result<Devices, Error>;

    /// Agora, em nanossegundos, no mesmo relógio dos pacotes.
    fn now_ns(&self) -> u64;

    /// Abre a trilha no dispositivo dado (`None` = o padrão do Windows): o microfone, ou o
    /// loopback da saída junto com o silêncio tocando nela.
    fn open(&self, track: Track, device: Option<&str>, events: Events) -> Result<Opened, Error>;
}

/// Segundos de áudio que o ring guarda: cobre a thread de escrita atrasada por uma
/// transcrição disputando a CPU.
pub const RING_SECS: u32 = 10;

/// O lado do callback.
pub struct Producer {
    samples: rtrb::Producer<f32>,
    packets: rtrb::Producer<Packet>,
    lost: Arc<AtomicU64>,
}

/// O lado da thread de escrita.
pub struct Consumer {
    samples: rtrb::Consumer<f32>,
    packets: rtrb::Consumer<Packet>,
    lost: Arc<AtomicU64>,
    pub format: Format,
}

/// Um ring para `secs` segundos no formato dado.
pub fn ring(format: Format, secs: u32) -> (Producer, Consumer) {
    let samples = format.rate as usize * usize::from(format.channels.max(1)) * secs as usize;
    // Um pacote por milissegundo é bem mais do que qualquer driver entrega.
    let (sp, sc) = rtrb::RingBuffer::new(samples.max(1));
    let (pp, pc) = rtrb::RingBuffer::new((secs as usize * 1_000).max(1));
    let lost = Arc::new(AtomicU64::new(0));
    (
        Producer {
            samples: sp,
            packets: pp,
            lost: lost.clone(),
        },
        Consumer {
            samples: sc,
            packets: pc,
            lost,
            format,
        },
    )
}

impl Producer {
    /// Copia um pacote de `len` amostras intercaladas. Sem espaço, o pacote inteiro é
    /// descartado e contado: o timestamp do próximo mostra o buraco.
    pub fn push(&mut self, t_ns: u64, frames: u32, samples: impl ExactSizeIterator<Item = f32>) {
        let len = samples.len();
        if self.samples.slots() < len || self.packets.slots() < 1 {
            self.lost.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // As amostras vão antes do cabeçalho: quem vê o cabeçalho já acha as amostras.
        if let Ok(chunk) = self.samples.write_chunk_uninit(len) {
            chunk.fill_from_iter(samples);
        }
        let _ = self.packets.push(Packet { t_ns, frames });
    }
}

impl Consumer {
    /// O próximo pacote, com as amostras intercaladas em `buf`.
    pub fn pop(&mut self, buf: &mut Vec<f32>) -> Option<Packet> {
        let packet = self.packets.pop().ok()?;
        let len = packet.frames as usize * usize::from(self.format.channels.max(1));
        buf.clear();
        if let Ok(chunk) = self.samples.read_chunk(len.min(self.samples.slots())) {
            let (a, b) = chunk.as_slices();
            buf.extend_from_slice(a);
            buf.extend_from_slice(b);
            chunk.commit_all();
        }
        Some(packet)
    }

    /// Pacotes descartados por falta de espaço.
    pub fn lost(&self) -> u64 {
        self.lost.load(Ordering::Relaxed)
    }
}

/// Por que a gravação não começou (ou o pedido não vale). O texto é para o usuário; o detalhe
/// técnico vai para o stderr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Já há uma gravação em andamento.
    Busy,
    /// Nenhuma gravação em andamento.
    NotRecording,
    NoMic,
    NoOutput,
    /// A privacidade do Windows bloqueou o microfone para apps da área de trabalho.
    MicDenied,
    /// O dispositivo existe, mas o stream não abriu.
    Device(Track),
    /// Pouco espaço para começar.
    Disk,
    /// Os arquivos da gravação não foram criados.
    Files,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => "Já há uma gravação em andamento.",
            Self::NotRecording => "Nenhuma gravação em andamento.",
            Self::NoMic => "Não encontrei um microfone. Conecte um e tente de novo.",
            Self::NoOutput => "Não encontrei uma saída de som para gravar o som do PC.",
            Self::MicDenied => {
                "O Windows bloqueou o microfone. Libere em Configurações > Privacidade e segurança > Microfone e tente de novo."
            }
            Self::Device(Track::Mic) => {
                "Não consegui abrir o microfone. Confira se ele está conectado e tente de novo."
            }
            Self::Device(Track::System) => {
                "Não consegui gravar o som do PC. Confira a saída de som e tente de novo."
            }
            Self::Disk => {
                "Pouco espaço em disco para gravar. Libere pelo menos 1 GB e tente de novo."
            }
            Self::Files => {
                "Não consegui criar os arquivos da gravação. Confira o espaço livre em disco e tente de novo."
            }
        })
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_entrega_na_ordem_e_conta_o_que_nao_coube() {
        let format = Format {
            rate: 100,
            channels: 2,
        };
        let (mut p, mut c) = ring(format, 1);
        p.push(10, 30, (0..60).map(|i| i as f32));
        p.push(20, 30, (60..120).map(|i| i as f32));
        // 200 amostras de espaço: o terceiro pacote não cabe.
        p.push(30, 50, (0..100).map(|i| i as f32));
        assert_eq!(c.lost(), 1);
        let mut buf = Vec::new();
        assert_eq!(
            c.pop(&mut buf),
            Some(Packet {
                t_ns: 10,
                frames: 30
            })
        );
        assert_eq!(buf.len(), 60);
        assert_eq!(buf[59], 59.0);
        assert_eq!(
            c.pop(&mut buf),
            Some(Packet {
                t_ns: 20,
                frames: 30
            })
        );
        assert_eq!(buf[0], 60.0);
        assert_eq!(c.pop(&mut buf), None);
        // Depois de ler, o espaço volta.
        p.push(40, 50, (0..100).map(|i| i as f32));
        assert_eq!(c.pop(&mut buf).map(|p| p.frames), Some(50));
    }

    #[test]
    fn mensagens_para_o_usuario() {
        assert_eq!(Error::Busy.to_string(), "Já há uma gravação em andamento.");
        assert!(Error::MicDenied.to_string().contains("Privacidade"));
        assert_ne!(
            Error::Device(Track::Mic).to_string(),
            Error::Device(Track::System).to_string()
        );
        assert_eq!(Track::Mic.file(), "mic.wav");
        assert_eq!(Track::System.index(), 1);
    }
}
