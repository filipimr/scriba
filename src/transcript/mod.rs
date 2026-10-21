//! A transcrição como texto: os segmentos do whisper viram parágrafos (uma pausa maior que
//! 1,5 s quebra o parágrafo), com `[hh:mm:ss]` no início de cada um e um cabeçalho com o
//! arquivo, a data e a duração. Sai em `.txt` (UTF-8, CRLF), que o leitor lê de volta com
//! [`parse`], e em `.md`.
//!
//! Numa gravação, cada parágrafo tem o falante ("Você", o microfone; "Outros", o som do PC)
//! depois do tempo, e o cabeçalho começa com `Gravação:`; só então o `parse` lê os rótulos,
//! e uma transcrição de arquivo nunca ganha rótulo. [`merge`] junta as duas trilhas e tira o
//! eco.

pub mod merge;
mod output;

pub use output::{
    Files, JOB_FILE, MANIFEST_VERSION, MD_FILE, Manifest, MediaKind, TXT_FILE, Timings, Written,
    clean_name, create_unique_dir, folder_name, write, write_atomic,
};

use crate::clock::LocalTime;

/// Pausa entre segmentos acima da qual começa um novo parágrafo.
pub const PAUSE_MS: u64 = 1_500;

/// Quem falou, numa gravação.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Speaker {
    /// O microfone.
    Voce,
    /// O som do PC.
    Outros,
}

impl Speaker {
    pub fn label(self) -> &'static str {
        match self {
            Self::Voce => "Você",
            Self::Outros => "Outros",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// Só numa gravação.
    pub speaker: Option<Speaker>,
}

impl Segment {
    /// Um segmento de arquivo, sem falante.
    pub fn new(start_ms: u64, end_ms: u64, text: impl Into<String>) -> Self {
        Self {
            start_ms,
            end_ms,
            text: text.into(),
            speaker: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paragraph {
    pub start_ms: u64,
    pub text: String,
    pub speaker: Option<Speaker>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Nome do arquivo original; numa gravação, "Gravação de 20/10, 14:30".
    pub file: String,
    pub date: LocalTime,
    pub duration_ms: u64,
    /// Uma gravação ao vivo: o cabeçalho diz as duas fontes e os parágrafos têm falante.
    pub recording: bool,
}

/// O que a pasta e o título usam no lugar do nome de arquivo numa gravação.
pub const RECORDING_NAME: &str = "Gravação";
/// As fontes, no cabeçalho de uma gravação.
pub const SPEAKERS: &str = "Você (microfone) e Outros (som do PC)";

impl Header {
    /// O nome que entra na pasta da saída (`AAAA-MM-DD HHmm - nome`).
    pub fn folder_base(&self) -> &str {
        if self.recording {
            RECORDING_NAME
        } else {
            &self.file
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    pub header: Header,
    pub paragraphs: Vec<Paragraph>,
}

impl Transcript {
    pub fn new(header: Header, segments: &[Segment]) -> Self {
        Self {
            header,
            paragraphs: paragraphs(segments),
        }
    }
}

/// Junta os segmentos em parágrafos. Segmentos sem texto são ignorados, e o espaço em
/// branco do texto vira um espaço só. A troca de falante também quebra o parágrafo.
pub fn paragraphs(segments: &[Segment]) -> Vec<Paragraph> {
    let mut out: Vec<Paragraph> = Vec::new();
    let mut last_end: Option<u64> = None;
    for segment in segments {
        let text = normalize(&segment.text);
        if text.is_empty() {
            continue;
        }
        match (out.last_mut(), last_end) {
            (Some(p), Some(end))
                if segment.start_ms.saturating_sub(end) <= PAUSE_MS
                    && p.speaker == segment.speaker =>
            {
                p.text.push(' ');
                p.text.push_str(&text);
            }
            _ => out.push(Paragraph {
                start_ms: segment.start_ms,
                text,
                speaker: segment.speaker,
            }),
        }
        last_end = Some(segment.end_ms);
    }
    out
}

/// Uma linha só: caracteres de controle e espaços repetidos viram um espaço.
pub fn normalize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `hh:mm:ss`, truncado ao segundo; as horas passam de 99 se precisar.
pub fn timestamp(ms: u64) -> String {
    let secs = ms / 1_000;
    format!("{:02}:{:02}:{:02}", secs / 3_600, secs / 60 % 60, secs % 60)
}

const FILE_KEY: &str = "Arquivo:";
const RECORDING_KEY: &str = "Gravação:";
const DATE_KEY: &str = "Data:";
const DURATION_KEY: &str = "Duração:";

/// `Você: ` ou `Outros: `, o começo do texto de um parágrafo de gravação.
fn speaker_prefix(speaker: Option<Speaker>) -> String {
    speaker.map_or_else(String::new, |s| format!("{}: ", s.label()))
}

/// O `.txt`: cabeçalho, linha em branco e um parágrafo por linha, separados por linhas em
/// branco. UTF-8 com CRLF, para o Bloco de Notas antigo.
pub fn render_txt(t: &Transcript) -> String {
    let first = if t.header.recording {
        format!("{RECORDING_KEY} {SPEAKERS}")
    } else {
        format!("{FILE_KEY} {}", normalize(&t.header.file))
    };
    let mut lines = vec![
        first,
        format!("{DATE_KEY} {}", t.header.date.display()),
        format!("{DURATION_KEY} {}", timestamp(t.header.duration_ms)),
    ];
    for p in &t.paragraphs {
        lines.push(String::new());
        lines.push(format!(
            "[{}] {}{}",
            timestamp(p.start_ms),
            speaker_prefix(p.speaker),
            p.text
        ));
    }
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

/// O `.md`: título com o nome do arquivo (ou "Gravação de" e a data), data e duração numa
/// lista e `**[hh:mm:ss]**` abrindo cada parágrafo (`**[hh:mm:ss] Você:**` numa gravação).
pub fn render_md(t: &Transcript) -> String {
    let title = if t.header.recording {
        format!("{RECORDING_NAME} de {}", t.header.date.display())
    } else {
        normalize(&t.header.file)
    };
    let mut out = format!(
        "# {}\n\n- Data: {}\n- Duração: {}\n",
        escape_md(&title),
        t.header.date.display(),
        timestamp(t.header.duration_ms)
    );
    if t.header.recording {
        out.push_str(&format!("- Falantes: {SPEAKERS}\n"));
    }
    for p in &t.paragraphs {
        let speaker = p.speaker.map_or_else(String::new, |s| format!(" {}:", s.label()));
        out.push_str(&format!(
            "\n**[{}]{speaker}** {}\n",
            timestamp(p.start_ms),
            escape_md(&p.text)
        ));
    }
    out
}

/// Escapa os caracteres que o Markdown interpretaria no meio do texto.
pub fn escape_md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// O `.txt` lido de volta, para o leitor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub file: Option<String>,
    pub date: Option<String>,
    pub duration: Option<String>,
    /// O cabeçalho começa com `Gravação:`: os parágrafos têm falante.
    pub recording: bool,
    pub blocks: Vec<Block>,
}

/// Um parágrafo do `.txt`. Texto fora do formato (sem `[hh:mm:ss]`, por exemplo editado à
/// mão) vem sem tempo e como está, com as quebras de linha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub start_ms: Option<u64>,
    pub text: String,
    /// Só numa gravação, e só se o parágrafo tem tempo e rótulo.
    pub speaker: Option<Speaker>,
}

pub fn parse(text: &str) -> Parsed {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut groups: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.split('\n').map(|l| l.trim_end_matches('\r')) {
        if line.trim().is_empty() {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }

    let mut parsed = Parsed::default();
    let mut groups = groups.into_iter().peekable();
    if let Some(first) = groups.peek()
        && first.iter().all(|l| header_line(l).is_some())
    {
        for line in first {
            match header_line(line) {
                Some((FILE_KEY, v)) => parsed.file = Some(v.to_string()),
                Some((RECORDING_KEY, _)) => parsed.recording = true,
                Some((DATE_KEY, v)) => parsed.date = Some(v.to_string()),
                Some((_, v)) => parsed.duration = Some(v.to_string()),
                None => {}
            }
        }
        groups.next();
    }
    for lines in groups {
        let (start_ms, first) = match split_timestamp(lines[0]) {
            Some((ms, rest)) => (Some(ms), rest),
            None => (None, lines[0]),
        };
        let (speaker, first) = match start_ms {
            Some(_) if parsed.recording => split_speaker(first),
            _ => (None, first),
        };
        let mut text = first.to_string();
        for line in &lines[1..] {
            text.push('\n');
            text.push_str(line);
        }
        parsed.blocks.push(Block {
            start_ms,
            text,
            speaker,
        });
    }
    parsed
}

fn header_line(line: &str) -> Option<(&'static str, &str)> {
    [FILE_KEY, RECORDING_KEY, DATE_KEY, DURATION_KEY]
        .into_iter()
        .find_map(|key| line.strip_prefix(key).map(|v| (key, v.trim())))
}

/// `Você: resto` → o falante e o resto.
fn split_speaker(text: &str) -> (Option<Speaker>, &str) {
    [Speaker::Voce, Speaker::Outros]
        .into_iter()
        .find_map(|s| {
            text.strip_prefix(s.label())
                .and_then(|r| r.strip_prefix(": "))
                .map(|rest| (Some(s), rest))
        })
        .unwrap_or((None, text))
}

/// `[hh:mm:ss] resto` → milissegundos e o resto.
fn split_timestamp(line: &str) -> Option<(u64, &str)> {
    let inner = line.strip_prefix('[')?;
    let (time, rest) = inner.split_once(']')?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let mut parts = time.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |p: &str, max_len: usize| {
        (p.len() >= 2 && p.len() <= max_len && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse::<u64>().ok())
            .flatten()
    };
    let (h, m, s) = (digits(h, 6)?, digits(m, 2)?, digits(s, 2)?);
    if parts.next().is_some() || m > 59 || s > 59 {
        return None;
    }
    Some(((h * 3_600 + m * 60 + s) * 1_000, rest))
}

#[cfg(test)]
mod tests;
