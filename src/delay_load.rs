//! O `vulkan-1.dll` com delay-load (`/DELAYLOAD`, no `build.rs`, com a feature `vulkan`). Sem
//! o hook, um PC sem driver de GPU (VM, Windows Sandbox) nem abriria o exe: o whisper.cpp
//! importa `vkGetInstanceProcAddr`, `vkGetPhysicalDeviceFeatures2` e `vkCmdCopyBuffer`.
//!
//! Quando a DLL falta, o [`failure_hook`] devolve o módulo do próprio exe no lugar dela; o
//! `GetProcAddress` então falha, e o hook devolve funções falsas: o `vkGetInstanceProcAddr`
//! falso dá, para qualquer nome, uma função que só retorna `VK_ERROR_INITIALIZATION_FAILED`.
//! O `ggml_vk_instance_init` lança na primeira chamada (`vkEnumerateInstanceVersion`), o
//! `catch` do `ggml_backend_vk_reg` devolve `nullptr`, e o ggml segue só com a CPU.
//!
//! A variável [`HIDE_VAR`] finge a falta da DLL (testes desta máquina, que tem driver).

use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

/// `SCRIBA_SEM_VULKAN=1`: age como se o `vulkan-1.dll` não existisse.
pub const HIDE_VAR: &str = "SCRIBA_SEM_VULKAN";

const VULKAN_DLL: &str = "vulkan-1.dll";
/// `VK_ERROR_INITIALIZATION_FAILED`.
pub const VK_INIT_FAILED: i32 = -3;

// Os códigos do `delayimp.h`.
const DLI_NOTE_PRE_LOAD_LIBRARY: u32 = 1;
const DLI_FAIL_LOAD_LIB: u32 = 3;
const DLI_FAIL_GET_PROC: u32 = 4;

/// `DelayLoadProc` do `delayimp.h`.
#[repr(C)]
pub struct DelayLoadProc {
    pub import_by_name: i32,
    /// `szProcName`, ou o ordinal na união.
    pub name: *const c_char,
}

/// `DelayLoadInfo` do `delayimp.h`.
#[repr(C)]
pub struct DelayLoadInfo {
    pub cb: u32,
    pub pidd: *const c_void,
    pub ppfn: *mut *const c_void,
    pub dll: *const c_char,
    pub proc_: DelayLoadProc,
    pub hmod_cur: *mut c_void,
    pub pfn_cur: *const c_void,
    pub last_error: u32,
}

/// `PfnDliHook`.
pub type Hook = Option<unsafe extern "system" fn(u32, *const DelayLoadInfo) -> *const c_void>;

/// O `vulkan-1.dll` faltou (ou foi escondido) e as funções falsas entraram.
static MISSING: AtomicBool = AtomicBool::new(false);

/// As funções do Vulkan são falsas nesta execução.
pub fn vulkan_missing() -> bool {
    MISSING.load(Ordering::Relaxed)
}

/// O `vulkan-1.dll` é achado pelo Windows (e não foi escondido por [`HIDE_VAR`]).
pub fn vulkan_present() -> bool {
    if hidden() {
        return false;
    }
    let name: Vec<u16> = VULKAN_DLL.encode_utf16().chain(Some(0)).collect();
    // SAFETY: o nome termina em zero; a DLL só é carregada para conferir e solta em seguida.
    unsafe {
        use windows_sys::Win32::Foundation::FreeLibrary;
        use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
        let module = LoadLibraryW(name.as_ptr());
        if module.is_null() {
            return false;
        }
        FreeLibrary(module);
    }
    true
}

fn hidden() -> bool {
    std::env::var_os(HIDE_VAR).is_some_and(|v| v == "1")
}

/// O nome da DLL do aviso é o `vulkan-1.dll` (sem diferenciar maiúsculas).
///
/// # Safety
/// `dll` é nulo ou aponta para uma string C válida.
unsafe fn is_vulkan(dll: *const c_char) -> bool {
    // SAFETY: garantido por quem chama.
    !dll.is_null()
        && unsafe { CStr::from_ptr(dll) }
            .to_bytes()
            .eq_ignore_ascii_case(VULKAN_DLL.as_bytes())
}

/// O módulo do exe: o `GetProcAddress` nele falha para os nomes do Vulkan, e o
/// [`failure_hook`] entrega as funções falsas.
fn own_module() -> *const c_void {
    // SAFETY: `GetModuleHandleW(NULL)` só lê o módulo do processo.
    unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) }
        .cast_const()
}

/// O `__pfnDliNotifyHook2`: com [`HIDE_VAR`], o `vulkan-1.dll` não é carregado.
///
/// # Safety
/// Chamado pelo `delayimp` com um `DelayLoadInfo` válido.
pub unsafe extern "system" fn notify_hook(
    notify: u32,
    info: *const DelayLoadInfo,
) -> *const c_void {
    // SAFETY: o `delayimp` passa um ponteiro válido.
    if notify == DLI_NOTE_PRE_LOAD_LIBRARY && hidden() && unsafe { is_vulkan((*info).dll) } {
        MISSING.store(true, Ordering::Relaxed);
        return own_module();
    }
    std::ptr::null()
}

/// O `__pfnDliFailureHook2`: o `vulkan-1.dll` que falta vira o módulo do exe, e cada função
/// dele, uma falsa. Outras DLLs seguem o comportamento padrão (a exceção do `delayimp`).
///
/// # Safety
/// Chamado pelo `delayimp` com um `DelayLoadInfo` válido.
pub unsafe extern "system" fn failure_hook(
    notify: u32,
    info: *const DelayLoadInfo,
) -> *const c_void {
    // SAFETY: o `delayimp` passa um ponteiro válido.
    let info = unsafe { &*info };
    // SAFETY: `dll` vem do descritor de importação, uma string C.
    if !unsafe { is_vulkan(info.dll) } {
        return std::ptr::null();
    }
    match notify {
        DLI_FAIL_LOAD_LIB => {
            MISSING.store(true, Ordering::Relaxed);
            own_module()
        }
        DLI_FAIL_GET_PROC => {
            MISSING.store(true, Ordering::Relaxed);
            if info.proc_.import_by_name != 0
                // SAFETY: importado pelo nome, `name` é uma string C.
                && unsafe { CStr::from_ptr(info.proc_.name) }.to_bytes() == b"vkGetInstanceProcAddr"
            {
                fake_get_instance_proc_addr as *const c_void
            } else {
                fail as *const c_void
            }
        }
        _ => std::ptr::null(),
    }
}

/// O `vkGetInstanceProcAddr` falso: toda função pedida falha.
unsafe extern "system" fn fake_get_instance_proc_addr(
    _instance: *const c_void,
    _name: *const c_char,
) -> *const c_void {
    fail as *const c_void
}

/// Qualquer função do Vulkan: no x64 do Windows quem chama limpa a pilha, então ignorar os
/// argumentos é seguro para qualquer assinatura.
unsafe extern "system" fn fail() -> i32 {
    VK_INIT_FAILED
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(dll: &CStr, proc_name: Option<&CStr>) -> DelayLoadInfo {
        DelayLoadInfo {
            cb: size_of::<DelayLoadInfo>() as u32,
            pidd: std::ptr::null(),
            ppfn: std::ptr::null_mut(),
            dll: dll.as_ptr(),
            proc_: DelayLoadProc {
                import_by_name: i32::from(proc_name.is_some()),
                name: proc_name.map_or(std::ptr::null(), CStr::as_ptr),
            },
            hmod_cur: std::ptr::null_mut(),
            pfn_cur: std::ptr::null(),
            last_error: 0,
        }
    }

    #[test]
    fn o_vulkan_que_falta_vira_funcoes_que_falham() {
        let load = info(c"VULKAN-1.dll", None);
        // SAFETY: os `DelayLoadInfo` deste teste são válidos.
        unsafe {
            assert_eq!(failure_hook(DLI_FAIL_LOAD_LIB, &load), own_module());
            let get = info(c"vulkan-1.dll", Some(c"vkGetInstanceProcAddr"));
            let proc_addr = failure_hook(DLI_FAIL_GET_PROC, &get);
            assert!(!proc_addr.is_null());
            type GetProc = unsafe extern "system" fn(*const c_void, *const c_char) -> *const c_void;
            let get_proc: GetProc = std::mem::transmute(proc_addr);
            let create = get_proc(std::ptr::null(), c"vkEnumerateInstanceVersion".as_ptr());
            type Enumerate = unsafe extern "system" fn(*mut u32) -> i32;
            let mut version = 0;
            let enumerate: Enumerate = std::mem::transmute(create);
            assert_eq!(enumerate(&mut version), VK_INIT_FAILED);
            assert_eq!(version, 0);

            let copy = info(c"vulkan-1.dll", Some(c"vkCmdCopyBuffer"));
            let f = failure_hook(DLI_FAIL_GET_PROC, &copy);
            let f: unsafe extern "system" fn() -> i32 = std::mem::transmute(f);
            assert_eq!(f(), VK_INIT_FAILED);
            // Por ordinal também.
            assert!(!failure_hook(DLI_FAIL_GET_PROC, &info(c"vulkan-1.dll", None)).is_null());
        }
        assert!(vulkan_missing());
    }

    #[test]
    fn outras_dlls_seguem_o_padrao() {
        // SAFETY: os `DelayLoadInfo` deste teste são válidos.
        unsafe {
            let other = info(c"outra.dll", Some(c"Funcao"));
            assert!(failure_hook(DLI_FAIL_LOAD_LIB, &other).is_null());
            assert!(failure_hook(DLI_FAIL_GET_PROC, &other).is_null());
            assert!(notify_hook(DLI_NOTE_PRE_LOAD_LIBRARY, &other).is_null());
            assert!(failure_hook(99, &info(c"vulkan-1.dll", None)).is_null());
            // Sem a variável, o pré-carregamento segue normal.
            if !hidden() {
                assert!(
                    notify_hook(DLI_NOTE_PRE_LOAD_LIBRARY, &info(c"vulkan-1.dll", None)).is_null()
                );
            }
        }
    }
}
