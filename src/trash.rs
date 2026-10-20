//! A Lixeira do Windows: remover um job concluído manda a pasta da saída para lá, de onde o
//! usuário ainda pode restaurá-la.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
    SHFileOperationW,
};

/// Manda o arquivo ou a pasta para a Lixeira, sem diálogo nem animação. Num volume sem
/// Lixeira, o Windows apaga de vez.
pub fn move_to_trash(path: &Path) -> io::Result<()> {
    // O shell não aceita caminho relativo nem o prefixo `\\?\`, e a lista termina com dois
    // nulos.
    let path = std::path::absolute(path)?;
    let mut from: Vec<u16> = path.as_os_str().encode_wide().collect();
    from.extend([0, 0]);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
        ..Default::default()
    };
    // SAFETY: `op` e a lista em `from` vivem até o fim da chamada, que é síncrona.
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 {
        return Err(io::Error::other(format!(
            "SHFileOperationW devolveu {code:#x}"
        )));
    }
    if op.fAnyOperationsAborted != 0 {
        return Err(io::Error::other("operação interrompida"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasta_vai_para_a_lixeira() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("2026-10-20 0900 - teste da lixeira");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("transcricao.txt"), "x").unwrap();
        move_to_trash(&target).unwrap();
        assert!(!target.exists());
        assert!(move_to_trash(&target).is_err(), "pasta que não existe");
    }
}
