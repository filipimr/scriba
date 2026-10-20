//! O log do whisper.cpp e do ggml: avisos e erros vão para o stderr, o resto é descartado.
//! As linhas que dizem qual GPU o whisper.cpp iniciou ficam registradas, por thread, para
//! conferir o candidato: quando a GPU não inicia, ele cai na CPU sem devolver erro.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_void};
use std::io::Write;
use std::sync::Once;

use whisper_rs::whisper_rs_sys as sys;

/// O que uma linha do log diz sobre a GPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gpu {
    /// `whisper_backend_init_gpu: using Vulkan1 backend`
    Using(String),
    /// `whisper_backend_init_gpu: failed to initialize Vulkan1 backend`
    Failed(String),
    /// `whisper_backend_init_gpu: no GPU found` (também numa CPU, que não pede GPU).
    NotFound,
}

const INIT_GPU: &str = "whisper_backend_init_gpu: ";

/// Classifica uma linha do log; `None` se ela não fala da GPU.
pub fn classify(line: &str) -> Option<Gpu> {
    let rest = line.trim().strip_prefix(INIT_GPU)?;
    if rest == "no GPU found" {
        return Some(Gpu::NotFound);
    }
    let named = |prefix: &str| {
        rest.strip_prefix(prefix)
            .and_then(|r| r.strip_suffix(" backend"))
            .map(str::to_string)
    };
    named("using ")
        .map(Gpu::Using)
        .or_else(|| named("failed to initialize ").map(Gpu::Failed))
}

/// Se a linha vai para o stderr: avisos e erros, e a continuação de uma linha mostrada.
pub fn visible(level: sys::ggml_log_level, previous: bool) -> bool {
    match level {
        sys::ggml_log_level_GGML_LOG_LEVEL_WARN | sys::ggml_log_level_GGML_LOG_LEVEL_ERROR => true,
        sys::ggml_log_level_GGML_LOG_LEVEL_CONT => previous,
        _ => false,
    }
}

thread_local! {
    static CAPTURED: RefCell<Option<Vec<Gpu>>> = const { RefCell::new(None) };
    static LAST_VISIBLE: Cell<bool> = const { Cell::new(false) };
}

/// Troca o log padrão do whisper.cpp (que também vale para o ggml) pelo filtro daqui.
pub fn install() {
    static ONCE: Once = Once::new();
    // SAFETY: a função vale pelo processo inteiro e não usa o ponteiro de dados.
    ONCE.call_once(|| unsafe { sys::whisper_log_set(Some(callback), std::ptr::null_mut()) });
}

/// Roda `f` e devolve também as linhas sobre a GPU que o whisper.cpp logou nesta thread.
pub fn capture<R>(f: impl FnOnce() -> R) -> (R, Vec<Gpu>) {
    install();
    CAPTURED.with(|c| *c.borrow_mut() = Some(Vec::new()));
    let result = f();
    let lines = CAPTURED.with(|c| c.borrow_mut().take()).unwrap_or_default();
    (result, lines)
}

unsafe extern "C" fn callback(level: sys::ggml_log_level, text: *const c_char, _: *mut c_void) {
    if text.is_null() {
        return;
    }
    // SAFETY: o whisper.cpp e o ggml passam uma string C válida durante a chamada.
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    if let Some(gpu) = classify(&text) {
        let _ = CAPTURED.try_with(|c| {
            if let Ok(mut c) = c.try_borrow_mut()
                && let Some(lines) = c.as_mut()
            {
                lines.push(gpu);
            }
        });
    }
    let show = LAST_VISIBLE
        .try_with(|last| {
            let show = visible(level, last.get());
            last.set(show);
            show
        })
        .unwrap_or(false);
    if show {
        // Nunca entra em pânico aqui: a chamada vem do C.
        let _ = std::io::stderr().write_all(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifica_as_linhas_da_gpu() {
        assert_eq!(
            classify("whisper_backend_init_gpu: using Vulkan1 backend\n"),
            Some(Gpu::Using("Vulkan1".into()))
        );
        assert_eq!(
            classify("whisper_backend_init_gpu: failed to initialize Vulkan0 backend\n"),
            Some(Gpu::Failed("Vulkan0".into()))
        );
        assert_eq!(
            classify("whisper_backend_init_gpu: no GPU found\n"),
            Some(Gpu::NotFound)
        );
        for other in [
            "whisper_backend_init_gpu: device 0: CPU (type: 0)\n",
            "whisper_backend_init_gpu: found GPU device 1: Vulkan1 (type: 1, cnt: 1)\n",
            "whisper_backend_init: using BLAS backend\n",
            "whisper_full_with_state: failed to encode\n",
            "",
        ] {
            assert_eq!(classify(other), None, "{other:?}");
        }
    }

    #[test]
    fn so_avisos_e_erros_aparecem() {
        use sys::*;
        assert!(visible(ggml_log_level_GGML_LOG_LEVEL_WARN, false));
        assert!(visible(ggml_log_level_GGML_LOG_LEVEL_ERROR, false));
        for quiet in [
            ggml_log_level_GGML_LOG_LEVEL_NONE,
            ggml_log_level_GGML_LOG_LEVEL_DEBUG,
            ggml_log_level_GGML_LOG_LEVEL_INFO,
        ] {
            assert!(!visible(quiet, true));
        }
        // A continuação segue a linha anterior.
        assert!(visible(ggml_log_level_GGML_LOG_LEVEL_CONT, true));
        assert!(!visible(ggml_log_level_GGML_LOG_LEVEL_CONT, false));
    }

    #[test]
    fn captura_so_na_propria_thread() {
        let ((), lines) = capture(|| {
            let msg = c"whisper_backend_init_gpu: using Vulkan1 backend\n";
            // SAFETY: string C válida; a função é a mesma que o whisper.cpp chamaria.
            unsafe {
                callback(
                    sys::ggml_log_level_GGML_LOG_LEVEL_INFO,
                    msg.as_ptr(),
                    std::ptr::null_mut(),
                );
            }
            std::thread::spawn(|| {
                let msg = c"whisper_backend_init_gpu: no GPU found\n";
                // SAFETY: idem.
                unsafe {
                    callback(
                        sys::ggml_log_level_GGML_LOG_LEVEL_INFO,
                        msg.as_ptr(),
                        std::ptr::null_mut(),
                    );
                }
            })
            .join()
            .unwrap();
        });
        assert_eq!(lines, [Gpu::Using("Vulkan1".into())]);
        // Fora de `capture`, nada é guardado.
        assert_eq!(capture(|| ()).1, []);
    }
}
