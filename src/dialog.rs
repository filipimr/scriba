//! "Guardar áudio": a janela "Salvar como" do Windows, aberta pelo exe. O app usa o
//! [`SystemDialog`]; os testes e o `--fake-pipeline` trocam por um que responde o que o teste
//! mandar.

use std::io;
use std::path::PathBuf;

/// O que a janela mostra ao abrir.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveRequest {
    /// O nome sugerido, com a extensão (`2026-10-20 1430 - Reunião.m4a`).
    pub file_name: String,
    /// A pasta em que a janela abre.
    pub dir: PathBuf,
}

pub trait SaveDialog: Send + Sync {
    /// Abre a janela e espera o usuário: o arquivo escolhido, ou `None` se ele cancelou.
    /// Bloqueia.
    fn ask(&self, request: &SaveRequest) -> io::Result<Option<PathBuf>>;
}

/// Título da janela.
pub const TITLE: &str = "Guardar áudio";

/// O caminho como o shell aceita: absoluto e só com `\` (com `/`, o
/// `SHCreateItemFromParsingName` recusa a pasta e a janela abre em Documentos).
fn shell_path(dir: &std::path::Path) -> PathBuf {
    std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// O `IFileSaveDialog` do Windows, numa thread própria com COM em STA.
pub struct SystemDialog;

impl SaveDialog for SystemDialog {
    fn ask(&self, request: &SaveRequest) -> io::Result<Option<PathBuf>> {
        let request = request.clone();
        std::thread::Builder::new()
            .name("scriba-salvar-como".into())
            .spawn(move || win::ask(&request))?
            .join()
            .map_err(|_| io::Error::other("a janela Salvar como entrou em pânico"))?
    }
}

mod win {
    use std::io;
    use std::path::PathBuf;

    use windows::Win32::Foundation::{ERROR_CANCELLED, HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
        CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FOS_PATHMUSTEXIST, FileSaveDialog,
        IFileSaveDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, CreateWindowExW, DestroyWindow, GetForegroundWindow,
        GetWindowThreadProcessId, SW_SHOWNA, SetForegroundWindow, ShowWindow, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_POPUP,
    };
    use windows::core::{HRESULT, HSTRING, w};

    use super::{SaveRequest, TITLE};

    pub fn ask(request: &SaveRequest) -> io::Result<Option<PathBuf>> {
        // SAFETY: a thread é nossa e acaba depois do CoUninitialize; tudo que é COM vive
        // dentro do `show`.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()?;
            let result = show(request);
            CoUninitialize();
            result
        }
    }

    unsafe fn show(request: &SaveRequest) -> io::Result<Option<PathBuf>> {
        unsafe {
            let dialog: IFileSaveDialog =
                CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetTitle(&HSTRING::from(TITLE))?;
            dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
                pszName: w!("Áudio M4A"),
                pszSpec: w!("*.m4a"),
            }])?;
            dialog.SetDefaultExtension(w!("m4a"))?;
            dialog.SetFileName(&HSTRING::from(request.file_name.as_str()))?;
            // `SetFolder`, e não `SetDefaultFolder`: a pasta lembrada pelo app vale mais que a
            // última que o Windows guardou. O shell recusa `/` no caminho (o `absolute` troca
            // por `\`). Pasta que sumiu: fica a do Windows.
            let dir = super::shell_path(&request.dir);
            let folder: windows::core::Result<IShellItem> =
                SHCreateItemFromParsingName(&HSTRING::from(dir.as_os_str()), None);
            match folder {
                Ok(folder) => dialog.SetFolder(&folder)?,
                Err(e) => eprintln!("a janela não abre em {}: {e}", dir.display()),
            }
            let options = dialog.GetOptions()?;
            dialog.SetOptions(
                options | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST,
            )?;

            let owner = Owner::create()?;
            let shown = dialog.Show(Some(owner.0));
            drop(owner);
            match shown {
                Ok(()) => {}
                Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => return Ok(None),
                Err(e) => return Err(e.into()),
            }
            let item = dialog.GetResult()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0 as *const _));
            Ok(Some(PathBuf::from(
                path.map_err(|e| io::Error::other(e.to_string()))?,
            )))
        }
    }

    /// A dona da janela: invisível (0 × 0), topmost e fora da barra de tarefas, no centro do
    /// monitor do navegador. Uma janela possuída por uma topmost também fica por cima; e,
    /// como o clique foi no navegador, o Windows nega o primeiro plano ao nosso processo:
    /// por um instante a fila de entrada da janela em primeiro plano vira a nossa
    /// (`AttachThreadInput`), e o `SetForegroundWindow` passa.
    struct Owner(HWND);

    impl Owner {
        unsafe fn create() -> io::Result<Self> {
            unsafe {
                let foreground = GetForegroundWindow();
                let monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTOPRIMARY);
                let mut info = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                let RECT {
                    left,
                    top,
                    right,
                    bottom,
                } = if GetMonitorInfoW(monitor, &mut info).as_bool() {
                    info.rcWork
                } else {
                    RECT::default()
                };
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                    w!("STATIC"),
                    &HSTRING::from(TITLE),
                    WS_POPUP,
                    (left + right) / 2,
                    (top + bottom) / 2,
                    0,
                    0,
                    None,
                    None,
                    None,
                    None,
                )?;
                let ours = GetCurrentThreadId();
                let theirs = GetWindowThreadProcessId(foreground, None);
                let attached = theirs != 0
                    && theirs != ours
                    && AttachThreadInput(theirs, ours, true).as_bool();
                let _ = ShowWindow(hwnd, SW_SHOWNA);
                let _ = SetForegroundWindow(hwnd);
                let _ = BringWindowToTop(hwnd);
                if attached {
                    let _ = AttachThreadInput(theirs, ours, false);
                }
                Ok(Self(hwnd))
            }
        }
    }

    impl Drop for Owner {
        fn drop(&mut self) {
            // SAFETY: a janela foi criada nesta thread e só é destruída aqui.
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::{Duration, Instant};

        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            FindWindowW, GWL_EXSTYLE, GetWindowLongW, PostMessageW, WM_CLOSE,
        };

        use super::*;
        use crate::dialog::SaveDialog;

        /// Local (`--ignored`): com outra janela em primeiro plano (o terminal do VS Code), a
        /// janela abre na frente, em primeiro plano ou topmost; fechá-la é cancelar.
        #[test]
        #[ignore = "desktop"]
        fn janela_abre_na_frente_e_fechar_cancela() {
            let watcher = std::thread::spawn(|| unsafe {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    if let Ok(hwnd) = FindWindowW(w!("#32770"), &HSTRING::from(TITLE)) {
                        // Dá tempo para a janela terminar de abrir.
                        std::thread::sleep(Duration::from_millis(500));
                        let foreground = GetForegroundWindow() == hwnd;
                        let topmost =
                            GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
                        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
                        return (foreground, topmost);
                    }
                    assert!(Instant::now() < deadline, "a janela não abriu");
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
            let answer = crate::dialog::SystemDialog
                .ask(&SaveRequest {
                    file_name: "2026-10-20 1430 - Teste.m4a".into(),
                    dir: std::env::temp_dir(),
                })
                .unwrap();
            let (foreground, topmost) = watcher.join().unwrap();
            println!("em primeiro plano: {foreground}; topmost: {topmost}");
            assert_eq!(answer, None);
            assert!(foreground || topmost);
        }
    }
}

/// Para o E2E (`--fake-pipeline`): imprime `salvar como: <pasta>|<nome>` no stdout e
/// responde com o conteúdo de `answer` (um caminho; vazio ou ausente é cancelar), que o
/// teste escreve antes de clicar.
#[cfg(any(test, feature = "test-support"))]
pub struct ScriptedDialog {
    answer: PathBuf,
}

#[cfg(any(test, feature = "test-support"))]
impl ScriptedDialog {
    pub const PREFIX: &'static str = "salvar como: ";
    /// O arquivo da resposta, em `<data-dir>`.
    pub const ANSWER_FILE: &'static str = "salvar-como.txt";

    pub fn new(data_dir: &std::path::Path) -> Self {
        Self {
            answer: data_dir.join(Self::ANSWER_FILE),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl SaveDialog for ScriptedDialog {
    fn ask(&self, request: &SaveRequest) -> io::Result<Option<PathBuf>> {
        use std::io::Write;
        {
            let mut out = io::stdout().lock();
            writeln!(
                out,
                "{}{}|{}",
                Self::PREFIX,
                request.dir.display(),
                request.file_name
            )?;
            out.flush()?;
        }
        let answer = std::fs::read_to_string(&self.answer).unwrap_or_default();
        let answer = answer.trim();
        Ok((!answer.is_empty()).then(|| PathBuf::from(answer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caminho_para_o_shell_so_tem_contrabarra() {
        assert_eq!(
            shell_path(std::path::Path::new(
                r"C:\Users\Ana\AppData\Local\Temp/scriba/saidas/2026-10-20 1430 - Reunião"
            )),
            PathBuf::from(
                r"C:\Users\Ana\AppData\Local\Temp\scriba\saidas\2026-10-20 1430 - Reunião"
            )
        );
        let relative = shell_path(std::path::Path::new("saidas/a"));
        assert!(relative.is_absolute(), "{}", relative.display());
        assert!(
            !relative.to_string_lossy().contains('/'),
            "{}",
            relative.display()
        );
    }

    #[test]
    fn roteiro_responde_o_arquivo_ou_cancela() {
        let dir = tempfile::tempdir().unwrap();
        let dialog = ScriptedDialog::new(dir.path());
        let request = SaveRequest {
            file_name: "a.m4a".into(),
            dir: dir.path().into(),
        };
        assert_eq!(dialog.ask(&request).unwrap(), None, "sem resposta");
        std::fs::write(dir.path().join(ScriptedDialog::ANSWER_FILE), "  \n").unwrap();
        assert_eq!(dialog.ask(&request).unwrap(), None, "vazia");
        let chosen = dir.path().join("Reunião.m4a");
        std::fs::write(
            dir.path().join(ScriptedDialog::ANSWER_FILE),
            format!("{}\r\n", chosen.display()),
        )
        .unwrap();
        assert_eq!(dialog.ask(&request).unwrap(), Some(chosen));
    }
}
