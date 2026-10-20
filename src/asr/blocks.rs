//! Blocos de ~30 min: uma reunião de 3 h não cabe inteira na memória (áudio, cópia só com a
//! fala e o mel do whisper). Cada bloco é uma janela do áudio cortada no meio de uma pausa
//! achada pelo VAD, perto do fim da janela; o próximo bloco começa no corte. Como os tempos
//! voltam ao original, a pausa na emenda é a pausa real, e o parágrafo quebra (ou não)
//! como quebraria sem blocos.

use std::ops::Range;
use std::time::Duration;

/// O tamanho padrão do bloco.
pub const BLOCK: Duration = Duration::from_secs(30 * 60);

/// A fração final da janela onde o corte pode cair: 1/6 (5 min num bloco de 30).
const SEARCH_DIVISOR: usize = 6;

/// Onde a região de busca começa numa janela de `len` amostras.
pub fn search_start(len: usize) -> usize {
    len - len / SEARCH_DIVISOR
}

/// Onde cortar uma janela de `len` amostras que não chega ao fim do áudio, dados os trechos
/// de fala dela (em ordem, sem sobreposição): no meio da maior pausa cujo meio cai na região
/// de busca, a mais tardia em caso de empate. Sem pausa na região, no meio da última pausa da
/// janela, para não cortar uma palavra. Uma janela sem fala corta no fim. `None` se a janela
/// não tem pausa nenhuma (música contínua): quem chama corta no fim, seco.
pub fn cut(spans: &[Range<usize>], len: usize) -> Option<usize> {
    if spans.is_empty() {
        return Some(len);
    }
    let from = search_start(len);
    let ends = std::iter::once(0).chain(spans.iter().map(|s| s.end));
    let starts = spans.iter().map(|s| s.start).chain(std::iter::once(len));
    let middle = |gap: &Range<usize>| gap.start + gap.len() / 2;
    let gaps: Vec<Range<usize>> = ends
        .zip(starts)
        .map(|(end, start)| end..start)
        .filter(|gap| !gap.is_empty() && middle(gap) > 0)
        .collect();
    let best = gaps.iter().filter(|gap| middle(gap) >= from).fold(
        None,
        |best: Option<&Range<usize>>, gap| match best {
            Some(b) if b.len() > gap.len() => Some(b),
            _ => Some(gap),
        },
    );
    best.or(gaps.last()).map(middle)
}

#[cfg(test)]
#[allow(
    clippy::single_range_in_vec_init,
    reason = "listas de trechos de fala com um trecho só"
)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn maior_pausa_na_regiao_de_busca() {
        // Janela de 600: a busca começa em 500.
        assert_eq!(search_start(600), 500);
        let spans = [0..100, 150..480, 520..540, 590..600];
        // Pausas: 480..520 (meio 500, 40) e 540..590 (meio 565, 50): vence a maior.
        assert_eq!(cut(&spans, 600), Some(565));
        // A pausa longa fora da região (100..150) não conta.
        let spans = [0..100, 400..560, 570..600];
        assert_eq!(cut(&spans, 600), Some(565));
    }

    #[test]
    fn empate_fica_com_a_mais_tardia() {
        let spans = [0..500, 520..560, 580..600];
        assert_eq!(cut(&spans, 600), Some(570));
    }

    #[test]
    fn silencio_no_fim_da_janela_conta_como_pausa() {
        assert_eq!(cut(&[0..520], 600), Some(560));
    }

    #[test]
    fn sem_pausa_na_regiao_vale_a_ultima_pausa_da_janela() {
        assert_eq!(cut(&[0..100, 200..300, 350..600], 600), Some(325));
        // Do começo da janela até o 1º trecho também é pausa.
        assert_eq!(cut(&[40..600], 600), Some(20));
    }

    #[test]
    fn sem_fala_corta_no_fim_e_sem_pausa_nao_ha_corte() {
        assert_eq!(cut(&[], 600), Some(600));
        assert_eq!(cut(&[0..600], 600), None);
    }

    /// Tamanho da janela e trechos de fala em ordem, sem sobreposição.
    fn case() -> impl Strategy<Value = (usize, Vec<Range<usize>>)> {
        (12usize..5_000).prop_flat_map(|len| {
            prop::collection::vec(0..=len, 0..30).prop_map(move |mut cuts| {
                cuts.sort_unstable();
                cuts.dedup();
                let spans = cuts
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|[a, b]| *a..*b)
                    .collect();
                (len, spans)
            })
        })
    }

    proptest! {
        #[test]
        fn corte_cai_numa_pausa_da_regiao_de_busca((len, spans) in case()) {
            // As pausas da janela e o meio de cada uma.
            let bounds: Vec<usize> = std::iter::once(0)
                .chain(spans.iter().flat_map(|s| [s.start, s.end]))
                .chain(std::iter::once(len))
                .collect();
            let middles: Vec<usize> = bounds
                .as_chunks::<2>()
                .0
                .iter()
                .filter(|[a, b]| a < b)
                .map(|[a, b]| a + (b - a) / 2)
                .filter(|&m| m > 0)
                .collect();
            match cut(&spans, len) {
                // Sem fala, o bloco é a janela inteira.
                Some(at) if spans.is_empty() => prop_assert_eq!(at, len),
                Some(at) => {
                    prop_assert!(middles.contains(&at), "{at} {middles:?}");
                    if middles.iter().any(|&m| m >= search_start(len)) {
                        prop_assert!(at >= search_start(len), "{at}");
                    }
                }
                None => prop_assert!(middles.is_empty()),
            }
        }
    }
}
