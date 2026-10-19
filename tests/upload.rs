//! Envio em streaming: com o exe real, um arquivo grande vai para o disco sem acumular na
//! memória, um envio recusado ainda recebe a resposta e um cliente que cai no meio não
//! deixa sobras; em processo, o encerramento não espera um envio parado.
//!
//! O envio grande tem `SCRIBA_UPLOAD_MB` megabytes (padrão 256). O critério da Fase 3, 2 GB
//! com a RAM estável, se mede com `SCRIBA_UPLOAD_MB=2048 cargo test --test upload -- --nocapture`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use scriba::config::Paths;
use scriba::jobs::Jobs;
use scriba::web::{AppState, bind, serve};
use tokio::io::AsyncWriteExt;

const MIB: u64 = 1024 * 1024;
const BOUNDARY: &str = "scriba-upload-teste";

/// O scriba.exe num diretório temporário; morto ao sair do escopo.
struct App {
    child: Child,
    port: u16,
    token: String,
    dir: tempfile::TempDir,
}

impl App {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_scriba"))
            .args(["--no-open", "--port", "0", "--data-dir"])
            .arg(dir.path())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        // "Scriba em http://127.0.0.1:<porta>/?t=<token>"
        let rest = line
            .trim()
            .strip_prefix("Scriba em http://127.0.0.1:")
            .expect(&line);
        let (port, token) = rest.split_once("/?t=").expect(&line);
        Self {
            child,
            port: port.parse().unwrap(),
            token: token.into(),
            dir,
        }
    }

    /// Tamanho de cada arquivo em `uploads/`.
    fn uploads(&self) -> Vec<u64> {
        uploads(self.dir.path())
    }

    fn get(&self, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\nCookie: scriba_token={t}\r\nConnection: close\r\n\r\n",
            p = self.port,
            t = self.token
        )
        .unwrap();
        let mut response = String::new();
        s.read_to_string(&mut response).unwrap();
        response
    }

    /// Abre o `POST /api/jobs` de um arquivo de `size` bytes e manda tudo o que vem antes
    /// dos bytes do arquivo.
    fn begin_upload(&self, file_name: &str, size: u64) -> TcpStream {
        let (head, tail) = multipart(file_name);
        let length = head.len() as u64 + size + tail.len() as u64;
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.write_all(post_head(self.port, &self.token, length).as_bytes())
            .unwrap();
        s.write_all(head.as_bytes()).unwrap();
        s
    }

    /// Envio completo; devolve a resposta HTTP inteira.
    fn upload(&self, file_name: &str, size: u64) -> String {
        let mut s = self.begin_upload(file_name, size);
        send_file_bytes(&mut s, size);
        s.write_all(multipart(file_name).1.as_bytes()).unwrap();
        let mut response = String::new();
        s.read_to_string(&mut response).unwrap();
        response
    }

    /// Pico do working set do processo, em bytes.
    fn peak_memory(&self) -> u64 {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        // SAFETY: estrutura C sem invariantes; zero é um valor válido.
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        // SAFETY: o handle é do processo filho, vivo enquanto `self` existe, e `counters`
        // tem o tamanho informado.
        let ok = unsafe { GetProcessMemoryInfo(self.child.as_raw_handle(), &mut counters, size) };
        assert_ne!(ok, 0, "GetProcessMemoryInfo falhou");
        counters.PeakWorkingSetSize as u64
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn uploads(data_dir: &std::path::Path) -> Vec<u64> {
    std::fs::read_dir(data_dir.join("uploads"))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.metadata().map_or(0, |m| m.len()))
                .collect()
        })
        .unwrap_or_default()
}

fn post_head(port: u16, token: &str, content_length: u64) -> String {
    format!(
        "POST /api/jobs HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nCookie: scriba_token={token}\r\nContent-Type: multipart/form-data; boundary={BOUNDARY}\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
    )
}

/// O multipart antes e depois dos bytes do arquivo, com a opção Transcrever.
fn multipart(file_name: &str) -> (String, String) {
    let head = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"transcrever\"\r\n\r\n1\r\n--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"arquivo\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    );
    (head, format!("\r\n--{BOUNDARY}--\r\n"))
}

/// Bytes 0..=250 em sequência, em blocos de 1 MiB: o limite do multipart nunca aparece
/// no meio do arquivo.
fn send_file_bytes(s: &mut TcpStream, size: u64) {
    let chunk: Vec<u8> = (0..MIB).map(|i| (i % 251) as u8).collect();
    let mut sent = 0;
    while sent < size {
        let n = (size - sent).min(MIB);
        s.write_all(&chunk[..n as usize]).unwrap();
        sent += n;
    }
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn envio_grande_vai_para_o_disco_sem_acumular_na_memoria() {
    let mb: u64 = std::env::var("SCRIBA_UPLOAD_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    let app = App::start();
    assert!(app.get("/").starts_with("HTTP/1.1 200"));
    let before = app.peak_memory();

    let t0 = Instant::now();
    // "lento": o pipeline falso demora, e o arquivo continua em uploads/ para a conferência.
    let response = app.upload("lento.mp4", mb * MIB);
    let elapsed = t0.elapsed();
    let after = app.peak_memory();
    assert!(
        response.starts_with("HTTP/1.1 200") && response.contains("lento.mp4"),
        "{}",
        &response[..response.len().min(400)]
    );
    let growth = after.saturating_sub(before);
    println!(
        "envio de {mb} MB em {elapsed:.1?} ({:.0} MB/s); pico de memória {} MB → {} MB (+{} MB)",
        mb as f64 / elapsed.as_secs_f64(),
        before / MIB,
        after / MIB,
        growth / MIB
    );
    assert!(growth < 100 * MIB, "a memória cresceu {} MB", growth / MIB);
    assert_eq!(app.uploads(), [mb * MIB]);
}

#[test]
fn envio_recusado_ainda_recebe_a_resposta() {
    let app = App::start();
    // O servidor lê o corpo até o fim antes de responder: o cliente não vê a conexão cair.
    let response = app.upload("nota.pdf", 32 * MIB);
    assert!(
        response.starts_with("HTTP/1.1 422") && response.contains("Formato não suportado"),
        "{}",
        &response[..response.len().min(400)]
    );
    assert!(app.uploads().is_empty());
}

#[test]
fn cliente_que_cai_no_meio_nao_deixa_arquivo_nem_job() {
    let app = App::start();
    let mut s = app.begin_upload("reuniao.mp4", 64 * MIB);
    send_file_bytes(&mut s, 8 * MIB);
    wait_until("o servidor começou a gravar", || app.uploads().len() == 1);
    drop(s);
    wait_until("o arquivo parcial ficou em uploads/", || {
        app.uploads().is_empty()
    });
    let list = app.get("/api/jobs");
    assert!(
        list.starts_with("HTTP/1.1 200") && !list.contains("job-card"),
        "{list}"
    );
}

#[tokio::test]
async fn encerramento_nao_espera_envio_parado() {
    let dir = tempfile::tempdir().unwrap();
    let listener = bind(0).unwrap();
    let port = listener.local_addr().unwrap().port();
    let paths = Paths::resolve(Some(dir.path().into()), Some(dir.path().join("saidas")));
    let jobs = Jobs::without_worker(paths.uploads_dir()).unwrap();
    let state = AppState::new("segredo".into(), "inst".into(), port, paths, jobs);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(
        listener,
        state,
        async {
            let _ = stopped.await;
        },
        Duration::from_millis(300),
    ));

    // Um envio que para no meio e fica esperando.
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    s.write_all(post_head(port, "segredo", 10 * MIB).as_bytes())
        .await
        .unwrap();
    s.write_all(multipart("parado.mp4").0.as_bytes())
        .await
        .unwrap();
    s.write_all(&[1u8; 64 * 1024]).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while uploads(dir.path()).is_empty() {
        assert!(Instant::now() < deadline, "o envio não começou");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    stop.send(()).unwrap();
    let t0 = Instant::now();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("o servidor não encerrou no prazo")
        .unwrap()
        .unwrap();
    assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
    drop(s);
}
