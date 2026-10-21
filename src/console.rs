//! O console das flags de linha de comando. A release é um app de janela
//! (`windows_subsystem = "windows"`): sem isto, `--self-test`, `--help`, `--version` e um erro
//! de argumento não mostrariam nada. O modo app nunca se anexa: um Ctrl+C ou fechar o
//! terminal mataria o processo no meio de uma gravação.

use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
    STD_OUTPUT_HANDLE, SetStdHandle,
};

/// Usa o console de quem chamou (o PowerShell, o cmd), se houver, para o stdout e o stderr
/// que ainda não vão para lugar nenhum. Um stdout redirecionado (`| Out-Host`, um arquivo)
/// continua onde está. No exe de console (debug), não muda nada.
pub fn attach_parent() {
    let missing = |h| std_handle(h).is_none();
    if !missing(STD_OUTPUT_HANDLE) && !missing(STD_ERROR_HANDLE) {
        return;
    }
    // SAFETY: sem console próprio, a chamada só se anexa ao do processo pai, se houver.
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        if missing(which)
            && let Some(console) = open_console_output()
        {
            // SAFETY: o handle é válido e fica aberto pelo resto do processo.
            unsafe { SetStdHandle(which, console) };
        }
    }
}

fn std_handle(which: STD_HANDLE) -> Option<HANDLE> {
    // SAFETY: só lê o handle padrão do processo.
    let h = unsafe { GetStdHandle(which) };
    (!h.is_null() && h != INVALID_HANDLE_VALUE).then_some(h)
}

fn open_console_output() -> Option<HANDLE> {
    let name: Vec<u16> = "CONOUT$".encode_utf16().chain(Some(0)).collect();
    // SAFETY: o nome termina em zero; o handle devolvido é nosso.
    let h = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE).then_some(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn com_os_handles_validos_nao_muda_nada() {
        // Sob o nextest, o stdout e o stderr são pipes: nada a anexar.
        let before = (std_handle(STD_OUTPUT_HANDLE), std_handle(STD_ERROR_HANDLE));
        attach_parent();
        assert_eq!(
            (std_handle(STD_OUTPUT_HANDLE), std_handle(STD_ERROR_HANDLE)),
            before
        );
    }
}
