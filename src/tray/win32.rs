//! A janela oculta e o ícone da bandeja (`Shell_NotifyIconW`, versão 4), numa thread
//! `scriba-bandeja`. A janela é top-level (não "message-only") para receber o
//! `WM_ENDSESSION`, o `TaskbarCreated` e o `WM_SETTINGCHANGE`; ela só traduz as mensagens em
//! [`TrayEvent`] e cuida do próprio ícone.

use std::cell::RefCell;
use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetMessageW, GetSystemMetrics, HICON, IMAGE_ICON,
    LR_DEFAULTCOLOR, LoadImageW, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage,
    RegisterClassExW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SetForegroundWindow,
    SetMenuDefaultItem, SetTimer, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, TranslateMessage, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY,
    WM_ENDSESSION, WM_NULL, WM_QUERYENDSESSION, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW,
    WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

use super::{END_SESSION_REASON, MENU_OPEN, MENU_QUIT, TrayEvent, TrayIcon, icon_for, tooltip};
use crate::shutdown::Shutdown;

/// A classe da janela oculta; os testes a acham por ela (e pelo processo).
pub const WINDOW_CLASS: &str = "ScribaBandeja";

/// Os ícones nos recursos do exe (`build.rs`).
const ICON_DARK: u16 = 2;
const ICON_LIGHT: u16 = 3;
const WM_TRAY: u32 = WM_APP + 1;
/// `NIN_SELECT | NINF_KEY`: Enter ou espaço no ícone.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const ICON_ID: u32 = 1;
const CMD_OPEN: usize = 1;
const CMD_QUIT: usize = 2;
const TIMER_TOOLTIP: usize = 1;
/// Quanto o `WM_ENDSESSION` espera o app fechar antes de deixar o Windows seguir.
const END_SESSION_WAIT: Duration = Duration::from_secs(20);

/// O que a janela precisa.
pub struct Options {
    /// Mostra o ícone; sem ele (`--no-tray`), a janela ainda recebe o fim da sessão.
    pub icon: bool,
    pub events: Sender<TrayEvent>,
    /// Há uma gravação em andamento (a dica muda).
    pub recording: Arc<dyn Fn() -> bool + Send + Sync>,
    pub shutdown: Shutdown,
}

/// A thread da bandeja; [`Tray::stop`] fecha a janela e tira o ícone.
pub struct Tray {
    hwnd: usize,
    thread: Option<JoinHandle<()>>,
}

impl Tray {
    pub fn start(opts: Options) -> io::Result<Self> {
        let (ready, created) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("scriba-bandeja".into())
            .spawn(move || match create(opts) {
                Ok(hwnd) => {
                    let _ = ready.send(Ok(hwnd as usize));
                    message_loop();
                }
                Err(e) => {
                    let _ = ready.send(Err(e));
                }
            })?;
        match created.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(io::Error::other(
                "a thread da bandeja terminou sem criar a janela",
            )),
        }
    }

    pub fn stop(mut self) {
        // SAFETY: a janela é desta thread da bandeja; se ela já fechou, a chamada falha.
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// O estado da janela, na thread dela.
struct Window {
    opts: Options,
    taskbar_created: u32,
    added: bool,
    recording: bool,
    icon: HICON,
}

thread_local! {
    static WINDOW: RefCell<Option<Window>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn create(opts: Options) -> io::Result<HWND> {
    let class = wide(WINDOW_CLASS);
    // SAFETY: as strings terminam em zero e vivem durante as chamadas; a classe é desta
    // thread e do exe.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        // Uma segunda bandeja no mesmo processo (testes) já acha a classe registrada.
        RegisterClassExW(&wc);
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let title = wide("Scriba");
        WINDOW.with_borrow_mut(|w| {
            *w = Some(Window {
                opts,
                taskbar_created,
                added: false,
                recording: false,
                icon: std::ptr::null_mut(),
            })
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            WINDOW.with_borrow_mut(|w| *w = None);
            return Err(io::Error::last_os_error());
        }
        SetTimer(hwnd, TIMER_TOOLTIP, 1000, None);
        with_window(|w| {
            if w.opts.icon {
                add_icon(w, hwnd);
            }
        });
        Ok(hwnd)
    }
}

fn message_loop() {
    let mut msg = MSG::default();
    // SAFETY: o loop padrão de mensagens da thread dona da janela.
    unsafe {
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    WINDOW.with_borrow_mut(|w| *w = None);
}

fn with_window<T>(f: impl FnOnce(&mut Window) -> T) -> Option<T> {
    // `try_borrow_mut`: o `TrackPopupMenuEx` e o `DestroyWindow` mandam mensagens à janela
    // enquanto ela está emprestada; essas caem no `DefWindowProcW`.
    WINDOW.with(|w| w.try_borrow_mut().ok()?.as_mut().map(f))
}

/// O tema da barra de tarefas, do registro.
fn system_uses_light_theme() -> Option<u32> {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("SystemUsesLightTheme");
    let (mut data, mut size) = (0u32, 4u32);
    // SAFETY: as strings terminam em zero e `data` tem os 4 bytes informados.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    (status == 0).then_some(data)
}

fn load_icon(icon: TrayIcon) -> HICON {
    let id = match icon {
        TrayIcon::Dark => ICON_DARK,
        TrayIcon::Light => ICON_LIGHT,
    };
    // SAFETY: `MAKEINTRESOURCE(id)`; um recurso que falta devolve nulo.
    unsafe {
        LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            id as usize as *const u16,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )
    }
}

fn data(hwnd: HWND, w: &Window) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_TRAY,
        hIcon: w.icon,
        ..Default::default()
    };
    let tip: Vec<u16> = tooltip(w.recording).encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    nid
}

fn refresh_icon(w: &mut Window) {
    let icon = load_icon(icon_for(system_uses_light_theme()));
    if !icon.is_null() {
        if !w.icon.is_null() {
            // SAFETY: o ícone anterior foi carregado por nós e o Explorer já tem a cópia dele.
            unsafe { DestroyIcon(w.icon) };
        }
        w.icon = icon;
    }
}

/// Põe o ícone (de novo, depois de o Explorer reiniciar).
fn add_icon(w: &mut Window, hwnd: HWND) {
    refresh_icon(w);
    let nid = data(hwnd, w);
    // SAFETY: `nid` está preenchido e vive durante as chamadas.
    unsafe {
        w.added = Shell_NotifyIconW(NIM_ADD, &nid) != 0;
        if w.added {
            Shell_NotifyIconW(NIM_SETVERSION, &nid);
        } else {
            eprintln!(
                "não consegui pôr o ícone na bandeja: {}",
                io::Error::last_os_error()
            );
        }
    }
}

fn modify_icon(w: &Window, hwnd: HWND) {
    if w.added {
        let nid = data(hwnd, w);
        // SAFETY: `nid` está preenchido e vive durante a chamada.
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }
}

fn send(w: &Window, event: TrayEvent) {
    let _ = w.opts.events.send(event);
}

/// O menu do clique direito; devolve o comando escolhido (0 = nenhum).
fn show_menu(hwnd: HWND, x: i32, y: i32) -> usize {
    let (open, quit) = (wide(MENU_OPEN), wide(MENU_QUIT));
    // SAFETY: o menu é criado, mostrado e destruído aqui; as strings vivem até o fim.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        AppendMenuW(menu, MF_STRING, CMD_OPEN, open.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, CMD_QUIT, quit.as_ptr());
        SetMenuDefaultItem(menu, CMD_OPEN as u32, 0);
        // Sem isso, o menu não fecha ao clicar fora dele.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            x,
            y,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd as usize
    }
}

/// `true` se `lparam` é a string "ImmersiveColorSet" (o tema mudou).
fn is_color_set(lparam: LPARAM) -> bool {
    if lparam == 0 {
        return false;
    }
    let target: Vec<u16> = "ImmersiveColorSet".encode_utf16().collect();
    // SAFETY: no `WM_SETTINGCHANGE`, um `lparam` não nulo é uma string terminada em zero.
    unsafe {
        let p = lparam as *const u16;
        (0..=target.len()).all(|i| {
            let c = *p.add(i);
            match target.get(i) {
                Some(t) => c == *t,
                None => c == 0,
            }
        })
    }
}

fn low_word(v: isize) -> u32 {
    (v as usize & 0xFFFF) as u32
}

fn signed_words(v: usize) -> (i32, i32) {
    (
        (v & 0xFFFF) as u16 as i16 as i32,
        ((v >> 16) & 0xFFFF) as u16 as i16 as i32,
    )
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = match msg {
        WM_TRAY => {
            let event = low_word(lparam);
            if event == NIN_SELECT || event == NIN_KEYSELECT {
                with_window(|w| send(w, TrayEvent::Open));
            } else if event == WM_CONTEXTMENU {
                let (x, y) = signed_words(wparam);
                match show_menu(hwnd, x, y) {
                    CMD_OPEN => with_window(|w| send(w, TrayEvent::Open)),
                    CMD_QUIT => with_window(|w| send(w, TrayEvent::Quit)),
                    _ => None,
                };
            }
            Some(0)
        }
        WM_TIMER if wparam == TIMER_TOOLTIP => {
            with_window(|w| {
                let now = (w.opts.recording)();
                if now != w.recording {
                    w.recording = now;
                    modify_icon(w, hwnd);
                }
            });
            Some(0)
        }
        WM_SETTINGCHANGE if is_color_set(lparam) => {
            with_window(|w| {
                if w.added {
                    refresh_icon(w);
                    modify_icon(w, hwnd);
                    let bar = match icon_for(system_uses_light_theme()) {
                        TrayIcon::Dark => "clara",
                        TrayIcon::Light => "escura",
                    };
                    eprintln!("bandeja: o tema mudou; ícone para a barra {bar}");
                }
            });
            Some(0)
        }
        WM_QUERYENDSESSION => Some(1),
        WM_ENDSESSION => {
            if wparam != 0 {
                // O Windows encerra o processo assim que esta mensagem volta: ela só volta
                // depois de a gravação e a fila fecharem.
                let reason = wide(END_SESSION_REASON);
                // SAFETY: a janela é esta; a string termina em zero.
                unsafe { ShutdownBlockReasonCreate(hwnd, reason.as_ptr()) };
                let shutdown = with_window(|w| {
                    send(w, TrayEvent::EndSession);
                    w.opts.shutdown.clone()
                });
                if let Some(shutdown) = shutdown
                    && !shutdown.wait_finished(END_SESSION_WAIT)
                {
                    eprintln!("o Scriba não terminou de fechar antes do fim da sessão");
                }
                // SAFETY: a janela é esta.
                unsafe { ShutdownBlockReasonDestroy(hwnd) };
            }
            Some(0)
        }
        WM_CLOSE => {
            // SAFETY: a janela é desta thread.
            unsafe { DestroyWindow(hwnd) };
            Some(0)
        }
        WM_DESTROY => {
            with_window(|w| {
                if w.added {
                    let nid = data(hwnd, w);
                    // SAFETY: `nid` está preenchido e vive durante as chamadas.
                    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
                    w.added = false;
                }
                if !w.icon.is_null() {
                    // SAFETY: o ícone foi carregado por nós.
                    unsafe { DestroyIcon(w.icon) };
                }
            });
            // SAFETY: encerra o loop desta thread.
            unsafe { PostQuitMessage(0) };
            Some(0)
        }
        m => with_window(|w| {
            // O Explorer reiniciou: o ícone volta. Se ele ainda estiver lá (a mensagem veio
            // sem reinício), o `NIM_ADD` falharia: sai antes.
            if m == w.taskbar_created && w.opts.icon {
                let nid = data(hwnd, w);
                // SAFETY: `nid` está preenchido e vive durante a chamada.
                unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
                add_icon(w, hwnd);
                if w.added {
                    eprintln!("bandeja: a barra de tarefas voltou; ícone reposto");
                }
                true
            } else {
                false
            }
        })
        .filter(|h| *h)
        .map(|_| 0),
    };
    // SAFETY: o tratamento padrão do Windows para o resto.
    handled.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palavras_do_wparam_com_sinal() {
        // Coordenadas negativas: um monitor à esquerda do principal.
        let v = ((-5i16 as u16 as usize) << 16) | (-120i16 as u16 as usize);
        assert_eq!(signed_words(v), (-120, -5));
        assert_eq!(low_word(0x0004_0400), 0x0400);
    }

    #[test]
    fn reconhece_a_mudanca_de_tema() {
        let s = wide("ImmersiveColorSet");
        assert!(is_color_set(s.as_ptr() as LPARAM));
        assert!(!is_color_set(wide("ImmersiveColorSetX").as_ptr() as LPARAM));
        assert!(!is_color_set(wide("Policy").as_ptr() as LPARAM));
        assert!(!is_color_set(0));
    }
}
