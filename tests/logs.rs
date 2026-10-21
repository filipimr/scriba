//! Os logs com o exe real (`--fake-pipeline`): o `scriba.log` recebe o stdout e o stderr, com
//! data e hora, e quem abriu o exe continua lendo os dois como antes; um pânico no worker fica
//! registrado com o lugar e a mensagem.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const BOUNDARY: &str = "scriba-logs-teste";

struct App {
    child: Child,
    port: u16,
    token: String,
    /// A primeira linha do stdout, lida antes de qualquer outra coisa.
    announce: String,
    stderr: Arc<Mutex<String>>,
    dir: tempfile::TempDir,
}

impl App {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_scriba"))
            .args(["--no-open", "--fake-pipeline", "--port", "0", "--data-dir"])
            .arg(dir.path())
            .arg("--output-dir")
            .arg(dir.path().join("saidas"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = Arc::new(Mutex::new(String::new()));
        {
            let stderr = stderr.clone();
            let mut pipe = child.stderr.take().unwrap();
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    stderr
                        .lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            });
        }
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let announce = line.trim().to_string();
        let rest = announce
            .strip_prefix("Scriba em http://127.0.0.1:")
            .expect(&announce);
        let (port, token) = rest.split_once("/?t=").expect(&announce);
        Self {
            port: port.parse().unwrap(),
            token: token.into(),
            announce: announce.clone(),
            child,
            stderr,
            dir,
        }
    }

    fn log_file(&self) -> PathBuf {
        self.dir.path().join("logs").join("scriba.log")
    }

    fn log(&self) -> String {
        read(&self.log_file())
    }

    /// Espera o log ter `text`; devolve o log.
    fn wait_log(&self, text: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let log = self.log();
            if log.contains(text) {
                return log;
            }
            assert!(Instant::now() < deadline, "o log não teve {text:?}:\n{log}");
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Envia um arquivo pequeno; devolve o status HTTP.
    fn upload(&self, file_name: &str) -> String {
        let body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"arquivo\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\nbytes falsos\r\n--{BOUNDARY}--\r\n"
        );
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(
            s,
            "POST /api/jobs HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nOrigin: http://127.0.0.1:{p}\r\nCookie: scriba_token={t}\r\nContent-Type: multipart/form-data; boundary={BOUNDARY}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            p = self.port,
            t = self.token,
            len = body.len()
        )
        .unwrap();
        let mut response = String::new();
        s.read_to_string(&mut response).unwrap();
        response.lines().next().unwrap_or_default().to_string()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read(path: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap_or_default()).into_owned()
}

/// `2026-10-21 14:30:05.123 `: a data e a hora na frente de cada linha.
fn stamped(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() > 24
        && b[..23].iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b' ',
            13 | 16 => *c == b':',
            19 => *c == b'.',
            _ => c.is_ascii_digit(),
        })
        && b[23] == b' '
}

#[test]
fn log_tem_a_abertura_e_quem_abriu_o_exe_ainda_le_tudo() {
    let app = App::start();
    let log = app.wait_log(&app.announce);
    let version = format!("Scriba {} · Windows 1", env!("CARGO_PKG_VERSION"));
    let log = if log.contains(&version) {
        log
    } else {
        app.wait_log(&version)
    };
    for line in log.lines() {
        assert!(stamped(line), "linha sem data e hora: {line:?}");
    }
    let opening = log.lines().find(|l| l.contains(&version)).unwrap();
    assert!(
        opening.contains("dados em ") && opening.contains("saídas em "),
        "{opening}"
    );
    // O stderr de quem abriu o exe recebe a mesma linha, sem a data na frente.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.stderr.lock().unwrap().contains(&version) {
        assert!(Instant::now() < deadline, "o stderr não recebeu a abertura");
        thread::sleep(Duration::from_millis(50));
    }
    let stderr = app.stderr.lock().unwrap().clone();
    assert!(stderr.lines().any(|l| l.starts_with(&version)), "{stderr}");
}

#[test]
fn panico_no_worker_fica_no_log() {
    let app = App::start();
    assert_eq!(app.upload("panico.mp3"), "HTTP/1.1 200 OK");
    let log = app.wait_log("pânico simulado pelo pipeline falso: panico.mp3");
    let line = log
        .lines()
        .find(|l| l.contains("pânico na thread"))
        .unwrap_or_else(|| panic!("sem a linha do pânico:\n{log}"));
    assert!(line.contains("src") && line.contains("fake.rs:"), "{line}");
    assert!(log.contains("backtrace:"), "{log}");
}
