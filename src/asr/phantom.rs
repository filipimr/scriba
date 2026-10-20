//! Frases-fantasma: o que o whisper "ouve" em silêncio, música ou ruído, herdado das legendas
//! com que foi treinado. O segmento sai inteiro quando é uma frase inequívoca. As ambíguas,
//! que também são fala de verdade ("Obrigado."), só saem quando o próprio whisper acha que
//! não havia fala.

/// A partir desta probabilidade de "sem fala", uma frase ambígua sai.
pub const NO_SPEECH: f32 = 0.5;

/// Frases ambíguas, já normalizadas: o segmento inteiro precisa ser uma delas.
const AMBIGUOUS: &[&str] = &[
    "obrigado por assistir",
    "obrigada por assistir",
    "obrigado",
    "obrigada",
    "muito obrigado",
    "muito obrigada",
    "tchau",
    "até a próxima",
    "até o próximo vídeo",
];

/// Se o segmento é uma frase-fantasma. `no_speech` é a probabilidade de "sem fala" que o
/// whisper deu à janela do segmento.
pub fn is_phantom(text: &str, no_speech: f32) -> bool {
    // Só marcações e símbolos: "[MÚSICA DE FUNDO]", "(risos)", "♪", "...".
    if !strip_marks(text).chars().any(char::is_alphanumeric) {
        return true;
    }
    let words = normalize(text);
    // Créditos de legenda e chamadas de canal de vídeo.
    if words.contains("amara org")
        || ((words.starts_with("inscreva se") || words.starts_with("se inscreva"))
            && words.contains("canal"))
    {
        return true;
    }
    no_speech >= NO_SPEECH && AMBIGUOUS.contains(&words.as_str())
}

/// Tira os trechos entre colchetes, parênteses e asteriscos.
fn strip_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut closer = None;
    for c in text.chars() {
        match closer {
            Some(end) if c == end => closer = None,
            Some(_) => {}
            None => match c {
                '[' => closer = Some(']'),
                '(' => closer = Some(')'),
                '*' => closer = Some('*'),
                _ => out.push(c),
            },
        }
    }
    out
}

/// Minúsculas, com tudo que não é letra nem número virando um espaço só.
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frases_inequivocas_saem_sempre() {
        for text in [
            " Legendas pela comunidade Amara.org",
            "Legendas pela comunidade do Amara.org.",
            " Inscreva-se no canal!",
            "Se inscreva no canal e ative o sininho.",
            "[MÚSICA DE FUNDO]",
            " (Música)",
            "[Aplausos] [risos]",
            "♪",
            " ♪♪ ",
            "*suspiro*",
            "...",
            "",
        ] {
            assert!(is_phantom(text, 0.0), "{text:?}");
        }
    }

    #[test]
    fn frases_ambiguas_saem_so_sem_fala() {
        for text in [
            " Obrigado por assistir!",
            "Obrigada.",
            "Muito obrigado.",
            " Tchau.",
            "Até a próxima!",
        ] {
            assert!(is_phantom(text, 0.8), "{text:?}");
            assert!(is_phantom(text, NO_SPEECH), "{text:?}");
            assert!(!is_phantom(text, 0.1), "{text:?}");
        }
    }

    #[test]
    fn fala_de_verdade_fica() {
        for text in [
            " Obrigado a todos pela presença na reunião de hoje.",
            "A reunião sobre o orçamento começa amanhã às nove horas.",
            "Inscreva-se no curso até sexta.",
            "(risos) Pois é, o prazo acabou.",
            "♪ Parabéns pra você ♪",
            "O site é amara ponto org?",
        ] {
            assert!(!is_phantom(text, 0.9), "{text:?}");
        }
    }

    #[test]
    fn marcacoes_saem_do_texto_conferido() {
        assert_eq!(strip_marks("a [b] c (d) *e* f"), "a  c   f");
        // Sem o fechamento, o resto é marcação.
        assert_eq!(strip_marks("fala [sem fim"), "fala ");
        assert_eq!(normalize(" Até-a  PRÓXIMA!!"), "até a próxima");
    }
}
