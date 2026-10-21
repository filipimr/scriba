//! O ciclo de vida do exe real (`--fake-pipeline`): a rota de saída e o fim da sessão do
//! Windows fecham com código 0 pelo mesmo caminho (a gravação em andamento fica nos WAVs e
//! volta na próxima abertura), e duas execuções na mesma pasta não sobem dois servidores.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ENDSESSION_LOGOFF, FindWindowExW, GetWindowThreadProcessId, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, WM_ENDSESSION, WM_QUERYENDSESSION,
};

/// O que o processo escreveu, juntado por uma thread.
type Output = Arc<Mutex<String>>;

fn collect(mut pipe: impl Read + Send + 'static) -> Output {
    let out = Output::default();
    let sink = out.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    out
}

fn spawn(dir: &Path, extra: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_scriba"))
        .args(["--no-open", "--no-tray", "--fake-pipeline", "--port", "0"])
        .args(extra)
        .arg("--data-dir")
        .arg(dir)
        .arg("--output-dir")
        .arg(dir.join("saidas"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

struct App {
    child: Child,
    port: u16,
    token: String,
    stdout: Output,
    stderr: Output,
}

impl App {
    /// Abre o exe e espera ele anunciar a URL.
    fn start(dir: &Path, extra: &[&str]) -> Self {
        Self::from(spawn(dir, extra))
    }

    fn from(mut child: Child) -> Self {
        let stderr = collect(child.stderr.take().unwrap());
        let stdout = collect(child.stdout.take().unwrap());
        let announce = wait_for(&stdout, "Scriba em ");
        let line = announce
            .lines()
            .find_map(|l| l.strip_prefix("Scriba em http://127.0.0.1:"))
            .unwrap()
            .trim()
            .to_string();
        let (port, token) = line.split_once("/?t=").expect(&line);
        Self {
            port: port.parse().unwrap(),
            token: token.into(),
            child,
            stdout,
            stderr,
        }
    }

    /// Uma requisição do app (Origin e cookie); devolve a resposta inteira.
    fn request(&self, method: &str, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(
            s,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:{p}\r\nCookie: scriba_token={t}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            p = self.port,
            t = self.token,
        )
        .unwrap();
        let mut response = String::new();
        s.read_to_string(&mut response).unwrap();
        response
    }

    fn post(&self, path: &str) -> String {
        let response = self.request("POST", path);
        response.lines().next().unwrap_or_default().to_string()
    }

    /// Espera o processo sair sozinho.
    fn wait_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "o exe não saiu:\n{}",
                self.stderr.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    /// Começa uma gravação (`--fake-audio`) e deixa gravar um pouco.
    fn record(&self) {
        assert_eq!(self.post("/api/record/start"), "HTTP/1.1 200 OK");
        thread::sleep(Duration::from_millis(1500));
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Espera `text` aparecer; devolve tudo o que veio.
fn wait_for(out: &Output, text: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let now = out.lock().unwrap().clone();
        if now.contains(text) {
            return now;
        }
        assert!(Instant::now() < deadline, "não veio {text:?}:\n{now}");
        thread::sleep(Duration::from_millis(50));
    }
}

/// A gravação que ficou em `gravacoes/` e quantas amostras cada trilha tem.
fn kept_recording(dir: &Path) -> (PathBuf, u32, u32) {
    let root = dir.join("gravacoes");
    let rec = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.is_dir())
        .expect("a gravação ficou em gravacoes/");
    let frames = |name| hound::WavReader::open(rec.join(name)).unwrap().duration();
    let (mic, system) = (frames("mic.wav"), frames("sistema.wav"));
    (rec, mic, system)
}

/// Fechou limpo: código 0 e o `instance.json` apagado.
fn closed_clean(app: &mut App, dir: &Path, reason: &str) {
    let status = app.wait_exit();
    let stderr = app.stderr();
    assert!(status.success(), "{status:?}\n{stderr}");
    assert!(!dir.join("instance.json").exists());
    assert!(
        stderr.contains(&format!("encerrando: {reason}")),
        "{stderr}"
    );
    assert!(stderr.contains("Scriba encerrado"), "{stderr}");
}

#[test]
fn rota_de_saida_fecha_com_codigo_zero() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::start(dir.path(), &[]);
    assert!(dir.path().join("instance.json").exists());
    assert_eq!(app.post("/api/app/sair"), "HTTP/1.1 204 No Content");
    closed_clean(&mut app, dir.path(), "rota de saída");
}

#[test]
fn rota_de_saida_exige_a_sessao() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(dir.path(), &[]);
    let mut s = TcpStream::connect(("127.0.0.1", app.port)).unwrap();
    write!(
        s,
        "POST /api/app/sair HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:{p}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        p = app.port
    )
    .unwrap();
    let mut response = String::new();
    s.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
    // Continua no ar.
    assert!(app.request("GET", "/health").starts_with("HTTP/1.1 200"));
}

#[test]
fn sair_gravando_guarda_os_wavs_e_a_reabertura_retoma() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::start(dir.path(), &["--fake-audio"]);
    app.record();
    assert_eq!(app.post("/api/app/sair"), "HTTP/1.1 204 No Content");
    closed_clean(&mut app, dir.path(), "rota de saída");
    assert!(
        app.stderr()
            .contains("gravação guardada para a próxima abertura")
    );
    let (rec, mic, system) = kept_recording(dir.path());
    // 32 kHz: mais de meio segundo nas duas trilhas, alinhadas no fim.
    assert!(mic > 16_000 && mic == system, "{mic} {system}");

    let mut again = App::start(dir.path(), &["--fake-audio"]);
    let stderr = wait_for(&again.stderr, "gravação retomada:");
    assert!(stderr.contains(&rec.display().to_string()), "{stderr}");
    let jobs = again.request("GET", "/api/jobs");
    assert!(jobs.contains("Gravação de"), "{jobs}");
    assert_eq!(again.post("/api/app/sair"), "HTTP/1.1 204 No Content");
    assert!(again.wait_exit().success());
}

#[test]
fn duas_execucoes_ao_mesmo_tempo_sobem_um_servidor() {
    let dir = tempfile::tempdir().unwrap();
    let mut children: Vec<Child> = (0..2).map(|_| spawn(dir.path(), &[])).collect();
    let outputs: Vec<(Output, Output)> = children
        .iter_mut()
        .map(|c| {
            (
                collect(c.stdout.take().unwrap()),
                collect(c.stderr.take().unwrap()),
            )
        })
        .collect();
    // A segunda espera a primeira responder e só anuncia a URL dela.
    let deadline = Instant::now() + Duration::from_secs(30);
    let second = loop {
        if let Some(i) = (0..2).find(|i| outputs[*i].0.lock().unwrap().contains("já está aberto"))
        {
            break i;
        }
        assert!(Instant::now() < deadline, "nenhuma execução cedeu");
        thread::sleep(Duration::from_millis(50));
    };
    let first = 1 - second;
    let status = loop {
        if let Some(status) = children[second].try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "a segunda execução não saiu");
        thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success(), "{status:?}");
    let served = wait_for(&outputs[first].0, "Scriba em ");
    let url = |text: &str, prefix: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(prefix).map(|u| u.trim().to_string()))
            .unwrap()
    };
    let ceded = outputs[second].0.lock().unwrap().clone();
    assert_eq!(
        url(&ceded, "Scriba já está aberto em "),
        url(&served, "Scriba em ")
    );
    // Só a que serve liga os logs e esvazia `uploads/`.
    let opening = format!("Scriba {} · ", env!("CARGO_PKG_VERSION"));
    assert!(!outputs[second].1.lock().unwrap().contains(&opening));
    let mut app = App {
        child: children.remove(first),
        port: 0,
        token: String::new(),
        stdout: outputs[first].0.clone(),
        stderr: outputs[first].1.clone(),
    };
    let line = url(&served, "Scriba em http://127.0.0.1:");
    let (port, token) = line.split_once("/?t=").unwrap();
    (app.port, app.token) = (port.parse().unwrap(), token.into());
    assert!(!app.stdout.lock().unwrap().is_empty());
    assert_eq!(app.post("/api/app/sair"), "HTTP/1.1 204 No Content");
    assert!(app.wait_exit().success());
}

/// A janela oculta do processo, achada pela classe.
fn hidden_window(pid: u32) -> HWND {
    let class: Vec<u16> = scriba::tray::WINDOW_CLASS
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let mut after: HWND = std::ptr::null_mut();
        loop {
            // SAFETY: a classe termina em zero; percorre as janelas top-level.
            after = unsafe {
                FindWindowExW(
                    std::ptr::null_mut(),
                    after,
                    class.as_ptr(),
                    std::ptr::null(),
                )
            };
            if after.is_null() {
                break;
            }
            let mut owner = 0;
            // SAFETY: só lê o processo dono da janela.
            unsafe { GetWindowThreadProcessId(after, &mut owner) };
            if owner == pid {
                return after;
            }
        }
        assert!(Instant::now() < deadline, "a janela oculta não apareceu");
        thread::sleep(Duration::from_millis(50));
    }
}

fn send(hwnd: HWND, msg: u32, wparam: usize, timeout_ms: u32) -> usize {
    let mut result = 0;
    // SAFETY: a janela é do processo filho; a mensagem não leva ponteiros.
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            msg,
            wparam,
            ENDSESSION_LOGOFF as isize,
            SMTO_ABORTIFHUNG,
            timeout_ms,
            &mut result,
        )
    };
    assert_ne!(ok, 0, "a janela não respondeu à mensagem {msg:#x}");
    result
}

#[test]
fn fim_da_sessao_guarda_a_gravacao_e_fecha_limpo() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::start(dir.path(), &["--fake-audio"]);
    app.record();
    let hwnd = hidden_window(app.child.id());
    assert_eq!(send(hwnd, WM_QUERYENDSESSION, 0, 5_000), 1);
    // Como no logoff: a mensagem só volta depois de o app fechar.
    send(hwnd, WM_ENDSESSION, 1, 30_000);
    let stderr = app.stderr();
    assert!(
        stderr.contains("gravação guardada para a próxima abertura"),
        "{stderr}"
    );
    closed_clean(&mut app, dir.path(), "fim da sessão do Windows");
    let (_, mic, system) = kept_recording(dir.path());
    assert!(mic > 16_000 && mic == system, "{mic} {system}");
}
