//! "Abrir pasta": mostra a pasta da saída no Explorer. O app usa o [`SystemOpener`]; os
//! testes e o `--fake-pipeline` trocam por um que só registra o pedido.

use std::io;
use std::path::Path;

pub trait Opener: Send + Sync {
    /// Abre a pasta sem esperar o programa que a mostra.
    fn open(&self, dir: &Path) -> io::Result<()>;
}

/// O Explorer, pelo crate `open`.
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, dir: &Path) -> io::Result<()> {
        open::that_detached(dir)
    }
}

/// Só imprime `abrir pasta: <dir>` no stdout, onde o E2E lê (`--fake-pipeline`).
#[cfg(any(test, feature = "test-support"))]
pub struct PrintOpener;

#[cfg(any(test, feature = "test-support"))]
impl Opener for PrintOpener {
    fn open(&self, dir: &Path) -> io::Result<()> {
        use std::io::Write;
        let mut out = io::stdout().lock();
        writeln!(out, "{}{}", Self::PREFIX, dir.display())?;
        out.flush()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PrintOpener {
    pub const PREFIX: &'static str = "abrir pasta: ";
}
