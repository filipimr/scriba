//! A gravação tem duas trilhas transcritas em separado: "Outros" (o som do PC) e "Você" (o
//! microfone). [`merge`] as junta por tempo e tira o eco: sem fone, o microfone também ouve
//! os alto-falantes, e a fala dos outros voltaria repetida como "Você".
//!
//! Um segmento "Você" é eco quando cai sobre segmentos "Outros" (com folga de
//! [`ECHO_WINDOW_MS`] para cada lado, pela latência e pela imprecisão dos tempos do whisper)
//! e pelo menos [`ECHO_SHARE`] das palavras dele aparecem, em ordem, no texto "Outros"
//! daquela janela. As palavras são comparadas sem caixa, pontuação nem acentos. "Outros"
//! nunca muda; fala sobreposta com palavras diferentes fica.
//!
//! Sem fone, o whisper às vezes entra em laço no eco e repete por minutos uma frase que
//! "Outros" disse uma vez, longe da janela. Por isso um "Você" longo (ao menos
//! [`LOOP_MIN_WORDS`] palavras) também é eco quando [`ECHO_SHARE`] das palavras dele estão,
//! em ordem, num só segmento "Outros" que começou até [`LOOP_LOOKBACK_MS`] antes dele, ou no
//! texto dos "Você" que começaram até [`REPEAT_LOOKBACK_MS`] antes do fim dele (o laço
//! repete a frase em pedaços que não coincidem com os segmentos de "Outros"). Uma resposta
//! curta da pessoa nunca cai nessas regras.

use super::{Segment, Speaker};

/// A folga, para cada lado, em que um segmento "Outros" conta como sobreposto.
pub const ECHO_WINDOW_MS: u64 = 1_000;
/// A fração das palavras de "Você" que, achadas em ordem em "Outros", fazem dele eco.
pub const ECHO_SHARE: f64 = 0.6;
/// O tamanho mínimo, em palavras, de um "Você" comparado com os "Outros" de antes.
pub const LOOP_MIN_WORDS: usize = 8;
/// Até quando, para trás, um "Outros" pode ter sido repetido pelo laço do whisper.
pub const LOOP_LOOKBACK_MS: u64 = 10 * 60_000;
/// Até quando, para trás, um "Você" longo repetido é o laço do whisper (os pedaços dele vêm
/// a cada 7–15 s; uma frase dita de novo meio minuto depois fica).
pub const REPEAT_LOOKBACK_MS: u64 = 30_000;

/// Junta as trilhas por tempo, com os rótulos, sem o eco em "Você". Os segmentos de cada
/// trilha vêm na ordem do whisper (por tempo); na saída, um "Outros" e um "Você" que começam
/// juntos ficam nessa ordem.
pub fn merge(outros: &[Segment], voce: &[Segment]) -> Vec<Segment> {
    let outros_words: Vec<Vec<String>> = outros.iter().map(|s| words(&s.text)).collect();
    let mut out: Vec<Segment> = outros
        .iter()
        .map(|s| with_speaker(s, Speaker::Outros))
        .collect();
    let voce_words: Vec<Vec<String>> = voce.iter().map(|s| words(&s.text)).collect();
    out.extend(
        voce.iter()
            .enumerate()
            .filter(|&(i, v)| !is_echo(v, outros, &outros_words) && !is_loop(i, voce, &voce_words))
            .map(|(_, v)| with_speaker(v, Speaker::Voce)),
    );
    // Estável: no empate, "Outros" (que veio antes) fica na frente.
    out.sort_by_key(|s| s.start_ms);
    out
}

fn with_speaker(segment: &Segment, speaker: Speaker) -> Segment {
    Segment {
        speaker: Some(speaker),
        ..segment.clone()
    }
}

fn is_echo(voce: &Segment, outros: &[Segment], outros_words: &[Vec<String>]) -> bool {
    let mine = words(&voce.text);
    if mine.is_empty() {
        return false;
    }
    let from = voce.start_ms.saturating_sub(ECHO_WINDOW_MS);
    let to = voce
        .end_ms
        .max(voce.start_ms)
        .saturating_add(ECHO_WINDOW_MS);
    let window: Vec<&str> = outros
        .iter()
        .zip(outros_words)
        .filter(|(o, _)| o.start_ms <= to && o.end_ms.max(o.start_ms) >= from)
        .flat_map(|(_, w)| w.iter().map(String::as_str))
        .collect();
    if !window.is_empty() && share(&mine, &window) >= ECHO_SHARE {
        return true;
    }
    mine.len() >= LOOP_MIN_WORDS
        && outros
            .iter()
            .zip(outros_words)
            .filter(|(o, _)| {
                o.start_ms <= to && o.start_ms.saturating_add(LOOP_LOOKBACK_MS) >= voce.start_ms
            })
            .any(|(_, w)| {
                let w: Vec<&str> = w.iter().map(String::as_str).collect();
                share(&mine, &w) >= ECHO_SHARE
            })
}

/// O "Você" `i` é longo e repete, em ordem, o texto dos "Você" dos 30 s anteriores.
fn is_loop(i: usize, voce: &[Segment], voce_words: &[Vec<String>]) -> bool {
    let mine = &voce_words[i];
    if mine.len() < LOOP_MIN_WORDS {
        return false;
    }
    // Pelo fim: depois de um silêncio, o whisper às vezes põe o começo do segmento no fim do
    // anterior, e a fala parece ter começado dezenas de segundos antes.
    let since = voce[i]
        .end_ms
        .max(voce[i].start_ms)
        .saturating_sub(REPEAT_LOOKBACK_MS);
    let before: Vec<&str> = voce[..i]
        .iter()
        .zip(voce_words)
        .filter(|(v, _)| v.start_ms >= since)
        .flat_map(|(_, w)| w.iter().map(String::as_str))
        .collect();
    !before.is_empty() && share(mine, &before) >= ECHO_SHARE
}

/// A fração das palavras de `voce` que aparecem em ordem em `outros` (0 a 1); texto sem
/// palavras dá 0.
pub fn similarity(voce: &str, outros: &str) -> f64 {
    let (mine, theirs) = (words(voce), words(outros));
    if mine.is_empty() {
        return 0.0;
    }
    let theirs: Vec<&str> = theirs.iter().map(String::as_str).collect();
    share(&mine, &theirs)
}

fn share(mine: &[String], theirs: &[&str]) -> f64 {
    lcs(mine, theirs) as f64 / mine.len() as f64
}

/// O tamanho da maior subsequência comum, em palavras.
fn lcs(a: &[String], b: &[&str]) -> usize {
    let mut prev = vec![0usize; b.len() + 1];
    let mut row = vec![0usize; b.len() + 1];
    for x in a {
        for (j, y) in b.iter().enumerate() {
            row[j + 1] = if x == y {
                prev[j] + 1
            } else {
                row[j].max(prev[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut row);
    }
    prev[b.len()]
}

/// As palavras do texto, em minúsculas e sem acentos; pontuação e espaços separam.
pub fn words(text: &str) -> Vec<String> {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(fold)
        .collect::<String>()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Tira o acento das letras do português (e das vizinhas que o whisper às vezes escreve).
fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        c => c,
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn seg(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment::new(start_ms, end_ms, text)
    }

    fn labels(segments: &[Segment]) -> Vec<(u64, Speaker, &str)> {
        segments
            .iter()
            .map(|s| (s.start_ms, s.speaker.unwrap(), s.text.as_str()))
            .collect()
    }

    #[test]
    fn palavras_sem_caixa_pontuacao_nem_acento() {
        assert_eq!(
            words("  Atenção: o ORÇAMENTO, já!  2027... "),
            ["atencao", "o", "orcamento", "ja", "2027"]
        );
        assert!(words("... — !").is_empty());
        assert_eq!(similarity("Bom dia, pessoal.", "bom dia pessoal"), 1.0);
        assert_eq!(similarity("bom dia", "dia bom"), 0.5);
        assert_eq!(similarity("", "qualquer"), 0.0);
        assert_eq!(
            similarity("um dois três quatro cinco", "um x dois y três"),
            0.6
        );
    }

    #[test]
    fn eco_sai_e_fala_propria_fica() {
        let outros = [
            seg(0, 3_000, " Vamos começar pela revisão do orçamento."),
            seg(6_000, 8_000, " Depois o cronograma."),
        ];
        let voce = [
            // O eco: o mesmo texto, 60 ms depois, com outra pontuação.
            seg(60, 3_100, " vamos começar pela revisao do orcamento"),
            // Fala por cima, com outras palavras.
            seg(1_000, 2_000, " Concordo, pode seguir."),
            // Sozinho.
            seg(4_000, 5_000, " Eu preparei a planilha."),
            // Eco com o tempo bem impreciso (900 ms antes).
            seg(5_100, 7_000, " depois o cronograma"),
        ];
        assert_eq!(
            labels(&merge(&outros, &voce)),
            [
                (
                    0,
                    Speaker::Outros,
                    " Vamos começar pela revisão do orçamento."
                ),
                (1_000, Speaker::Voce, " Concordo, pode seguir."),
                (4_000, Speaker::Voce, " Eu preparei a planilha."),
                (6_000, Speaker::Outros, " Depois o cronograma."),
            ]
        );
    }

    #[test]
    fn fora_da_janela_nao_e_eco() {
        let outros = [seg(0, 1_000, "bom dia")];
        // Começa 1.001 ms depois do fim: o mesmo texto, mas não é eco.
        let voce = [seg(2_001, 3_000, "bom dia")];
        assert_eq!(merge(&outros, &voce).len(), 2);
        // Uma trilha vazia.
        assert_eq!(
            labels(&merge(&[], &voce)),
            [(2_001, Speaker::Voce, "bom dia")]
        );
        assert_eq!(
            labels(&merge(&outros, &[])),
            [(0, Speaker::Outros, "bom dia")]
        );
    }

    #[test]
    fn laco_do_whisper_no_eco_sai() {
        let frase =
            " Volto a dizer, por fim, ministro, caso a flexibilização resulte em acidentes.";
        let outros = [
            seg(450_000, 455_000, frase),
            seg(500_000, 503_000, " Outro assunto."),
        ];
        let voce = [
            // O laço: a frase de "Outros" de novo, minutos depois e com outra pontuação.
            seg(
                530_000,
                535_000,
                " volto a dizer por fim ministro caso a flexibilizacao resulte em acidentes",
            ),
            // Longa, mas fala própria.
            seg(
                540_000,
                545_000,
                " Eu preparei a planilha com os números do trimestre passado.",
            ),
            // Curta e igual a um pedaço de "Outros": fica (só as longas entram na regra).
            seg(560_000, 561_000, " Volto a dizer."),
            // A frase de novo, mas mais de 10 min depois: fica.
            seg(1_051_000, 1_056_000, frase),
            // O laço continua, inteiro e em pedaços rotacionados: saem, pela repetição dentro de
            // 30 s.
            seg(1_056_000, 1_060_000, frase),
            seg(
                1_060_000,
                1_064_000,
                " ministro, caso a flexibilização resulte em acidentes. Volto a dizer, por fim,",
            ),
            seg(1_200_000, 1_201_000, " Combinado."),
            seg(1_210_000, 1_211_000, " Combinado."),
            // Uma frase longa da pessoa dita de novo 40 s depois: fica, mesmo com o começo
            // posto pelo whisper no fim da anterior.
            seg(
                1_300_000,
                1_304_000,
                " Eu envio o relatório por email amanhã cedo, combinado?",
            ),
            seg(
                1_304_000,
                1_344_000,
                " Eu envio o relatório por email amanhã cedo, combinado?",
            ),
        ];
        assert_eq!(
            labels(&merge(&outros, &voce))
                .iter()
                .filter(|(_, s, _)| *s == Speaker::Voce)
                .map(|(t, _, _)| *t)
                .collect::<Vec<_>>(),
            [
                540_000, 560_000, 1_051_000, 1_200_000, 1_210_000, 1_300_000, 1_304_000
            ]
        );
    }

    #[test]
    fn empate_poe_outros_antes() {
        let merged = merge(&[seg(500, 900, "a")], &[seg(500, 900, "b")]);
        assert_eq!(
            labels(&merged),
            [(500, Speaker::Outros, "a"), (500, Speaker::Voce, "b")]
        );
    }

    const VOCAB: [&str; 12] = [
        "orçamento",
        "reunião",
        "amanhã",
        "você",
        "cronograma",
        "três",
        "é",
        "planilha",
        "começar",
        "atenção",
        "projeto",
        "dia",
    ];

    fn text() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::sample::select(VOCAB.to_vec()), 1..8)
            .prop_map(|w| format!(" {}.", w.join(" ")))
    }

    /// Segmentos em ordem de tempo, como o whisper entrega.
    fn track() -> impl Strategy<Value = Vec<Segment>> {
        prop::collection::vec((0u64..4_000, 0u64..5_000, text()), 0..10).prop_map(|parts| {
            let mut t = 0;
            parts
                .into_iter()
                .map(|(gap, len, text)| {
                    let start = t + gap;
                    t = start + len;
                    Segment::new(start, t, text)
                })
                .collect()
        })
    }

    /// O mesmo texto como o whisper poderia escrever de novo: outra caixa, sem acentos, sem
    /// pontuação ou com outra.
    fn rewrite(text: &str, style: u8) -> String {
        match style % 4 {
            0 => text.to_uppercase(),
            1 => text.chars().map(fold).collect(),
            2 => words(text).join(" "),
            _ => text.replace(' ', ", ").replace('.', "?"),
        }
    }

    proptest! {
        #[test]
        fn mescla_e_eco(
            outros in track(),
            voce in track(),
            copy in any::<prop::sample::Index>(),
            shift in -1_000i64..=1_000,
            style in any::<u8>(),
        ) {
            // Mais um "Você": a cópia de um "Outros", deslocada e reescrita.
            let mut all_voce = voce.clone();
            let copied = (!outros.is_empty()).then(|| {
                let o = &outros[copy.index(outros.len())];
                let at = |ms: u64| (ms as i64 + shift).max(0) as u64;
                Segment::new(at(o.start_ms), at(o.end_ms), rewrite(&o.text, style))
            });
            all_voce.extend(copied.clone());
            let merged = merge(&outros, &all_voce);

            // Ordenada por tempo.
            prop_assert!(merged.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
            // "Outros" intacto: todos, na mesma ordem, só com o rótulo.
            let kept_outros: Vec<Segment> = merged
                .iter()
                .filter(|s| s.speaker == Some(Speaker::Outros))
                .map(|s| Segment { speaker: None, ..s.clone() })
                .collect();
            prop_assert_eq!(&kept_outros, &outros);
            let kept_voce: Vec<&Segment> =
                merged.iter().filter(|s| s.speaker == Some(Speaker::Voce)).collect();
            prop_assert_eq!(kept_outros.len() + kept_voce.len(), merged.len());
            // "Você" sem "Outros" na janela fica.
            for v in &voce {
                let alone = outros.iter().all(|o| {
                    o.end_ms + ECHO_WINDOW_MS < v.start_ms
                        || o.start_ms > v.end_ms + ECHO_WINDOW_MS
                });
                if alone {
                    let found = kept_voce
                        .iter()
                        .any(|k| k.start_ms == v.start_ms && k.text == v.text);
                    prop_assert!(found, "{v:?} sumiu");
                }
            }
            // A cópia sai.
            if let Some(c) = copied {
                let found = kept_voce
                    .iter()
                    .any(|k| k.start_ms == c.start_ms && k.text == c.text);
                prop_assert!(!found, "o eco {c:?} ficou");
            }
        }
    }
}
