//! Quanto uma transcrição deve levar: a duração do áudio vezes o fator de tempo real (RTF)
//! medido no benchmark da etapa 4.3, mais a carga do modelo e, no 1º uso do Vulkan no
//! processo, a compilação dos shaders. A extração do áudio fica de fora até ser medida.

use super::backend::{Backend, Plan};
use crate::jobs::Estimate;

/// Carga do modelo, quando ele não está na memória (≤ 1,2 s em todos no benchmark).
pub const LOAD_SECS: f64 = 1.2;
/// Compilação dos shaders no 1º uso do Vulkan em cada processo (~9 s na RTX 3050).
pub const SHADERS_SECS: f64 = 9.0;

/// RTF do benchmark (45,5 min de FLEURS pt_br) por backend e modelo: a RTX 3050 vale para
/// qualquer GPU dedicada e o i7-13650HX para qualquer CPU.
pub fn rtf(gpu: bool, model: &str) -> f64 {
    match (gpu, model) {
        (true, "base") => 0.016,
        (true, "small") => 0.020,
        (true, _) => 0.026,
        (false, "base") => 0.046,
        (false, "small") => 0.105,
        (false, _) => 0.350,
    }
}

/// A estimativa para `audio_ms` de áudio no candidato `plan` com o modelo `model`.
/// `loaded`: o modelo já está na memória; `warmed`: o Vulkan já rodou neste processo.
pub fn estimate(plan: &Plan, model: &str, audio_ms: u64, loaded: bool, warmed: bool) -> Estimate {
    let gpu = matches!(plan.backend, Backend::Gpu(_));
    let mut secs = audio_ms as f64 / 1_000.0 * rtf(gpu, model);
    if !loaded {
        secs += LOAD_SECS;
    }
    if gpu && !warmed {
        secs += SHADERS_SECS;
    }
    Estimate {
        secs: secs.ceil() as u64,
        gpu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::backend::{Device, DeviceKind};
    use crate::asr::models::ModelKind;

    fn rtx() -> Plan {
        Plan {
            backend: Backend::Gpu(Device {
                index: 1,
                name: "Vulkan1".into(),
                description: "NVIDIA GeForce RTX 3050 6GB Laptop GPU".into(),
                kind: DeviceKind::Dedicated,
                memory: 6 << 30,
            }),
            model: ModelKind::Large,
            flash_attn: true,
        }
    }

    const HOUR: u64 = 3_600_000;

    #[test]
    fn uma_hora_na_gpu_e_na_cpu() {
        // 3600 × 0,026 = 93,6 s, mais carga e shaders.
        let first = estimate(&rtx(), "large-v3-turbo", HOUR, false, false);
        assert_eq!(
            first,
            Estimate {
                secs: 104,
                gpu: true
            }
        );
        // Modelo carregado e shaders prontos: só o RTF.
        assert_eq!(
            estimate(&rtx(), "large-v3-turbo", HOUR, true, true).secs,
            94
        );
        // Descarregado depois de ocioso: volta a carga, não os shaders.
        assert_eq!(
            estimate(&rtx(), "large-v3-turbo", HOUR, false, true).secs,
            95
        );

        // 3600 × 0,105 = 378 s.
        let cpu = estimate(&Plan::cpu(), "small", HOUR, false, false);
        assert_eq!(
            cpu,
            Estimate {
                secs: 380,
                gpu: false
            }
        );
        assert_eq!(estimate(&Plan::cpu(), "base", HOUR, true, false).secs, 166);
    }

    #[test]
    fn audio_vazio_custa_so_a_carga() {
        assert_eq!(estimate(&Plan::cpu(), "small", 0, false, false).secs, 2);
        assert_eq!(estimate(&Plan::cpu(), "small", 0, true, false).secs, 0);
    }

    #[test]
    fn modelo_desconhecido_usa_o_rtf_do_large() {
        assert_eq!(rtf(false, "medium"), rtf(false, "large-v3-turbo"));
        assert_eq!(rtf(true, "medium"), rtf(true, "large-v3-turbo"));
    }
}
