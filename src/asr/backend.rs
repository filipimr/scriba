//! Onde transcrever: as GPUs que o ggml enxerga e a ordem em que o app tenta usá-las. GPUs
//! dedicadas primeiro (mais memória antes), depois a integrada e, por fim, a CPU com o
//! modelo menor.

use std::cmp::Reverse;
use std::ffi::{CStr, c_char};
use std::fmt;

use whisper_rs::whisper_rs_sys as sys;

use super::log::{self, Gpu};
use super::models::{ModelKind, Models};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Dedicated,
    Integrated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// O `gpu_device` do whisper.cpp, que conta as GPUs dedicadas e integradas na ordem do
    /// registro do ggml.
    pub index: i32,
    /// Nome no ggml, o mesmo do log do whisper.cpp (`Vulkan1`).
    pub name: String,
    /// Nome da placa (`NVIDIA GeForce RTX 3050 6GB Laptop GPU`).
    pub description: String,
    pub kind: DeviceKind,
    /// Memória total, em bytes.
    pub memory: u64,
}

/// As GPUs do registro do ggml, na ordem dele. Sem a feature `vulkan`, nenhuma.
pub fn devices() -> Vec<Device> {
    // Antes do registro: a enumeração do Vulkan loga cada placa.
    log::install();
    let mut out = Vec::new();
    // SAFETY: o registro do ggml é estático e os ponteiros valem pelo processo inteiro.
    unsafe {
        for i in 0..sys::ggml_backend_dev_count() {
            let dev = sys::ggml_backend_dev_get(i);
            let kind = match sys::ggml_backend_dev_type(dev) {
                sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU => DeviceKind::Dedicated,
                sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU => DeviceKind::Integrated,
                _ => continue,
            };
            let (mut free, mut total) = (0, 0);
            sys::ggml_backend_dev_memory(dev, &mut free, &mut total);
            out.push(Device {
                index: out.len() as i32,
                name: text(sys::ggml_backend_dev_name(dev)),
                description: text(sys::ggml_backend_dev_description(dev)),
                kind,
                memory: total as u64,
            });
        }
    }
    out
}

/// Os backends compilados no ggml (`CPU`, e `Vulkan` com a feature `vulkan`).
pub fn registry() -> Vec<String> {
    log::install();
    // SAFETY: como em `devices`.
    unsafe {
        (0..sys::ggml_backend_reg_count())
            .map(|i| text(sys::ggml_backend_reg_name(sys::ggml_backend_reg_get(i))))
            .collect()
    }
}

/// A memória livre agora na GPU, em bytes; `None` se ela sumiu do registro.
pub fn free_memory(device: &Device) -> Option<u64> {
    // SAFETY: como em `devices`.
    unsafe {
        (0..sys::ggml_backend_dev_count())
            .map(|i| sys::ggml_backend_dev_get(i))
            .find(|&dev| text(sys::ggml_backend_dev_name(dev)) == device.name)
            .map(|dev| {
                let (mut free, mut total) = (0, 0);
                sys::ggml_backend_dev_memory(dev, &mut free, &mut total);
                free as u64
            })
    }
}

/// # Safety
/// `p` é nulo ou aponta para uma string C válida.
unsafe fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: garantido por quem chama.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Backend {
    Gpu(Device),
    Cpu,
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gpu(d) => write!(f, "{} ({})", d.description, d.name),
            Self::Cpu => f.write_str("CPU"),
        }
    }
}

/// Um candidato: onde transcrever, com qual modelo e se usa flash attention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub backend: Backend,
    pub model: ModelKind,
    pub flash_attn: bool,
}

impl Plan {
    pub fn cpu() -> Self {
        Self {
            backend: Backend::Cpu,
            model: ModelKind::Cpu,
            flash_attn: false,
        }
    }

    /// Confere, pelas linhas do log, se o whisper.cpp iniciou a GPU planejada. Numa falha
    /// ele cai na CPU sem devolver erro, o que levaria o modelo grande para a CPU.
    pub fn check(&self, lines: &[Gpu]) -> Result<(), String> {
        let Backend::Gpu(device) = &self.backend else {
            return Ok(());
        };
        if let Some(Gpu::Failed(name)) = lines.iter().find(|l| matches!(l, Gpu::Failed(_))) {
            return Err(format!("o whisper.cpp não iniciou a {name}"));
        }
        if lines.contains(&Gpu::Using(device.name.clone())) {
            Ok(())
        } else {
            Err(format!("o whisper.cpp não usou a {}", device.name))
        }
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let model = match self.model {
            ModelKind::Large => "modelo grande",
            ModelKind::Cpu => "modelo da CPU",
        };
        write!(f, "{}, {model}", self.backend)?;
        if self.flash_attn {
            f.write_str(", flash attention")?;
        }
        Ok(())
    }
}

/// Os candidatos, em ordem: GPUs dedicadas (mais memória primeiro) com o modelo grande e
/// flash attention e, por fim, a CPU com o modelo dela. A GPU integrada fica de fora: no
/// benchmark da etapa 4.3 ela foi mais lenta que a CPU com todos os modelos. Um candidato
/// só entra se o modelo dele está disponível.
pub fn plans(devices: &[Device], models: &Models) -> Vec<Plan> {
    let mut gpus: Vec<&Device> = devices
        .iter()
        .filter(|d| d.kind == DeviceKind::Dedicated)
        .collect();
    // Estável: entre iguais, vale a ordem do registro.
    gpus.sort_by_key(|d| Reverse(d.memory));
    gpus.into_iter()
        .map(|d| Plan {
            backend: Backend::Gpu(d.clone()),
            model: ModelKind::Large,
            flash_attn: true,
        })
        .chain([Plan::cpu()])
        .filter(|p| models.get(p.model).is_some())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::models::{Model, Source};

    fn gpu(index: i32, kind: DeviceKind, gb: u64) -> Device {
        Device {
            index,
            name: format!("Vulkan{index}"),
            description: format!("placa {index}"),
            kind,
            memory: gb << 30,
        }
    }

    fn models(large: bool, cpu: bool) -> Models {
        let model = |name| Model {
            name,
            source: Source::Embedded(b""),
        };
        Models {
            large: large.then(|| model("large-v3-turbo")),
            cpu: cpu.then(|| model("base")),
            vad: Some(Source::Embedded(b"")),
        }
    }

    fn summary(plans: &[Plan]) -> Vec<(Option<i32>, ModelKind, bool)> {
        plans
            .iter()
            .map(|p| {
                let index = match &p.backend {
                    Backend::Gpu(d) => Some(d.index),
                    Backend::Cpu => None,
                };
                (index, p.model, p.flash_attn)
            })
            .collect()
    }

    #[test]
    fn integrada_e_rtx_usam_so_a_rtx() {
        // Como nesta máquina: a Intel é o dispositivo 0 e a RTX, o 1.
        let devices = [
            gpu(0, DeviceKind::Integrated, 8),
            gpu(1, DeviceKind::Dedicated, 6),
        ];
        let plans = plans(&devices, &models(true, true));
        assert_eq!(
            summary(&plans),
            [
                (Some(1), ModelKind::Large, true),
                (None, ModelKind::Cpu, false),
            ]
        );
    }

    #[test]
    fn so_integrada_usa_a_cpu() {
        let plans = plans(&[gpu(0, DeviceKind::Integrated, 4)], &models(true, true));
        assert_eq!(plans, [Plan::cpu()]);
    }

    #[test]
    fn sem_gpu_usa_a_cpu() {
        assert_eq!(plans(&[], &models(true, true)), [Plan::cpu()]);
    }

    #[test]
    fn duas_dedicadas_preferem_a_de_mais_memoria() {
        let devices = [
            gpu(0, DeviceKind::Dedicated, 4),
            gpu(1, DeviceKind::Dedicated, 12),
            gpu(2, DeviceKind::Dedicated, 4),
        ];
        let order: Vec<_> = summary(&plans(&devices, &models(true, true)))
            .into_iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(order, [Some(1), Some(0), Some(2), None]);
    }

    #[test]
    fn candidato_sem_modelo_fica_de_fora() {
        let devices = [
            gpu(0, DeviceKind::Integrated, 8),
            gpu(1, DeviceKind::Dedicated, 6),
        ];
        // Sem o modelo grande (o release só com o modelo da CPU), só sobra o que usa o da CPU.
        let expected: Vec<_> = summary(&plans(&devices, &models(true, true)))
            .into_iter()
            .filter(|s| s.1 == ModelKind::Cpu)
            .collect();
        assert_eq!(summary(&plans(&devices, &models(false, true))), expected);
        // Sem o modelo da CPU, a CPU sai da lista.
        assert!(
            plans(&devices, &models(true, false))
                .iter()
                .all(|p| p.backend != Backend::Cpu)
        );
        assert!(plans(&[], &models(false, false)).is_empty());
    }

    #[test]
    fn confere_a_gpu_pelo_log() {
        let rtx = Plan {
            backend: Backend::Gpu(gpu(1, DeviceKind::Dedicated, 6)),
            model: ModelKind::Large,
            flash_attn: true,
        };
        assert!(rtx.check(&[Gpu::Using("Vulkan1".into())]).is_ok());
        // Falhou ao iniciar, não achou a GPU ou iniciou outra: o whisper.cpp caiu na CPU.
        for lines in [
            vec![Gpu::Failed("Vulkan1".into())],
            vec![Gpu::NotFound],
            vec![Gpu::Using("Vulkan0".into())],
            vec![],
            vec![Gpu::Using("Vulkan1".into()), Gpu::Failed("Vulkan1".into())],
        ] {
            assert!(rtx.check(&lines).is_err(), "{lines:?}");
        }
        // Na CPU o log não importa.
        assert!(Plan::cpu().check(&[Gpu::NotFound]).is_ok());
    }

    #[test]
    fn descreve_o_candidato() {
        let rtx = Plan {
            backend: Backend::Gpu(gpu(1, DeviceKind::Dedicated, 6)),
            model: ModelKind::Large,
            flash_attn: true,
        };
        assert_eq!(
            rtx.to_string(),
            "placa 1 (Vulkan1), modelo grande, flash attention"
        );
        assert_eq!(Plan::cpu().to_string(), "CPU, modelo da CPU");
    }
}
