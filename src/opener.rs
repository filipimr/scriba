//! "Abrir pasta" e "Mostrar no Explorer": mostram a pasta da saída, ou o áudio guardado
//! selecionado na pasta dele, no Explorer. O app usa o [`SystemOpener`]; os testes e o
//! `--fake-pipeline` trocam por um que só registra o pedido.

use std::io;
use std::path::Path;

pub trait Opener: Send + Sync {
    /// Abre a pasta sem esperar o programa que a mostra.
    fn open(&self, dir: &Path) -> io::Result<()>;

    /// Abre a pasta do arquivo com ele selecionado.
    fn reveal(&self, file: &Path) -> io::Result<()>;
}

/// O Explorer, pelo crate `open` e pelo shell do Windows.
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, dir: &Path) -> io::Result<()> {
        open::that_detached(dir)
    }

    fn reveal(&self, file: &Path) -> io::Result<()> {
        use windows::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
        };
        use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};
        use windows::core::HSTRING;

        // O shell não aceita caminho relativo.
        let file = std::path::absolute(file)?;
        // SAFETY: o COM é iniciado e encerrado nesta chamada (numa thread que já o tinha, o
        // par só ajusta a contagem); o ITEMIDLIST vive até o `ILFree`.
        unsafe {
            let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            let pidl = ILCreateFromPathW(&HSTRING::from(file.as_os_str()));
            let result = if pidl.is_null() {
                Err(io::Error::from(io::ErrorKind::NotFound))
            } else {
                let shown = SHOpenFolderAndSelectItems(pidl, None, 0).map_err(io::Error::from);
                ILFree(Some(pidl));
                shown
            };
            if com.is_ok() {
                CoUninitialize();
            }
            result
        }
    }
}

/// Só imprime `abrir pasta: <dir>` e `mostrar arquivo: <arquivo>` no stdout, onde o E2E lê
/// (`--fake-pipeline`).
#[cfg(any(test, feature = "test-support"))]
pub struct PrintOpener;

#[cfg(any(test, feature = "test-support"))]
impl Opener for PrintOpener {
    fn open(&self, dir: &Path) -> io::Result<()> {
        Self::print(Self::PREFIX, dir)
    }

    fn reveal(&self, file: &Path) -> io::Result<()> {
        Self::print(Self::REVEAL_PREFIX, file)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PrintOpener {
    pub const PREFIX: &'static str = "abrir pasta: ";
    pub const REVEAL_PREFIX: &'static str = "mostrar arquivo: ";

    fn print(prefix: &str, path: &Path) -> io::Result<()> {
        use std::io::Write;
        let mut out = io::stdout().lock();
        writeln!(out, "{prefix}{}", path.display())?;
        out.flush()
    }
}
