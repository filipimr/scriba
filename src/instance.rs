//! Instância única: a instância em execução se registra em `instance.json`; uma segunda
//! execução confere se ela responde e, nesse caso, só reabre o navegador nela.

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    /// Identifica a instância no `/health`; não é segredo.
    pub id: String,
    pub port: u16,
    /// Segredo da sessão: só quem lê este arquivo (o próprio usuário) abre a UI.
    pub token: String,
}

impl Instance {
    pub fn new(port: u16) -> Self {
        Self {
            id: random_hex(8),
            port,
            token: random_hex(32),
        }
    }

    /// URL de entrada; o servidor troca o token da query por um cookie.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/?t={}", self.port, self.token)
    }

    /// Arquivo ausente ou corrompido conta como "nenhuma instância".
    pub fn read(path: &Path) -> Option<Self> {
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self)?)
    }

    /// Remove o arquivo só se ele ainda for desta instância.
    pub fn remove(&self, path: &Path) {
        if Self::read(path).is_some_and(|i| i.id == self.id) {
            let _ = fs::remove_file(path);
        }
    }

    /// A instância responde no `/health` com o mesmo id? Um processo morto ou outro
    /// programa na porta dá `false`.
    pub fn is_alive(&self) -> bool {
        probe(self.port).is_some_and(|id| id == self.id)
    }
}

/// O mutex nomeado da pasta de dados: quem o cria é a instância que serve. Fica com o
/// processo até ele terminar (o Windows o solta mesmo num crash).
pub struct Lock(HANDLE);

impl Drop for Lock {
    fn drop(&mut self) {
        // SAFETY: o handle é nosso e não foi fechado.
        unsafe { CloseHandle(self.0) };
    }
}

pub enum Claim {
    /// Esta execução serve.
    Mine(Lock),
    /// Outra execução na mesma pasta de dados já serve (ou está abrindo).
    Taken,
}

/// Tenta ser a instância da pasta de dados. Duas aberturas ao mesmo tempo não sobem dois
/// servidores (os dois esvaziariam `uploads/`); pastas diferentes (os E2E em paralelo) não
/// se atrapalham.
pub fn claim(data_dir: &Path) -> std::io::Result<Claim> {
    let name: Vec<u16> = mutex_name(data_dir).encode_utf16().chain(Some(0)).collect();
    // SAFETY: o nome termina em zero; o handle devolvido é nosso.
    let (handle, error) = unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        (h, GetLastError())
    };
    if handle.is_null() {
        return Err(std::io::Error::from_raw_os_error(error as i32));
    }
    let lock = Lock(handle);
    Ok(if error == ERROR_ALREADY_EXISTS {
        Claim::Taken
    } else {
        Claim::Mine(lock)
    })
}

/// `Local\Scriba-<hash>`: o hash do caminho absoluto, sem diferença de maiúsculas (o NTFS
/// não diferencia) nem da barra no fim.
pub fn mutex_name(data_dir: &Path) -> String {
    let path = std::path::absolute(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let text = path.to_string_lossy().to_lowercase().replace('/', "\\");
    format!(
        "Local\\Scriba-{:016x}",
        fnv1a(text.trim_end_matches('\\').as_bytes())
    )
}

/// FNV-1a de 64 bits: estável entre versões do Rust (o `DefaultHasher` não é).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Espera até `timeout` a instância de `path` responder no `/health` (a outra execução pode
/// estar abrindo agora).
pub fn wait_alive(path: &Path, timeout: Duration) -> Option<Instance> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(found) = Instance::read(path).filter(Instance::is_alive) {
            return Some(found);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `GET /health` em 127.0.0.1:port; devolve o corpo (o id da instância).
fn probe(port: u16) -> Option<String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let timeout = Duration::from_millis(1500);
    let mut stream = TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    write!(
        stream,
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.take(4096).read_to_string(&mut response).ok()?;
    let (head, body) = response.split_once("\r\n\r\n")?;
    head.starts_with("HTTP/1.1 200")
        .then(|| body.trim().to_string())
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("gerador aleatório do sistema");
    hex::encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_sao_aleatorios_e_longos() {
        let (a, b) = (Instance::new(1), Instance::new(1));
        assert_eq!(a.token.len(), 64);
        assert_ne!(a.token, b.token);
        assert_ne!(a.id, b.id);
        assert!(a.url().starts_with("http://127.0.0.1:1/?t="));
    }

    #[test]
    fn arquivo_ausente_ou_corrompido_e_nenhuma_instancia() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("instance.json");
        assert_eq!(Instance::read(&path), None);
        fs::write(&path, "lixo").unwrap();
        assert_eq!(Instance::read(&path), None);
    }

    #[test]
    fn escreve_le_e_so_remove_o_proprio_arquivo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("instance.json");
        let (mine, other) = (Instance::new(10), Instance::new(11));
        mine.write(&path).unwrap();
        assert_eq!(Instance::read(&path), Some(mine.clone()));
        other.remove(&path);
        assert!(path.exists());
        mine.remove(&path);
        assert!(!path.exists());
    }

    #[test]
    fn mutex_por_pasta_de_dados() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let first = claim(&a).unwrap();
        assert!(matches!(first, Claim::Mine(_)));
        assert!(matches!(claim(&a).unwrap(), Claim::Taken));
        // A mesma pasta escrita de outro jeito é a mesma instância.
        let upper = format!("{}\\", a.to_string_lossy().to_uppercase());
        assert!(matches!(claim(Path::new(&upper)).unwrap(), Claim::Taken));
        assert!(matches!(claim(&b).unwrap(), Claim::Mine(_)));
        drop(first);
        assert!(matches!(claim(&a).unwrap(), Claim::Mine(_)));
    }

    #[test]
    fn nome_do_mutex() {
        let name = mutex_name(Path::new(r"C:\Dados\Scriba"));
        assert!(name.starts_with(r"Local\Scriba-"), "{name}");
        assert_eq!(name.len(), r"Local\Scriba-".len() + 16);
        assert_eq!(name, mutex_name(Path::new("c:/dados/scriba/")));
        assert_ne!(name, mutex_name(Path::new(r"C:\Dados\Outro")));
    }

    #[test]
    fn espera_quem_nao_responde_ate_o_prazo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("instance.json");
        let started = Instant::now();
        assert_eq!(wait_alive(&path, Duration::from_millis(250)), None);
        assert!(started.elapsed() >= Duration::from_millis(250));
    }

    #[test]
    fn porta_sem_servidor_nao_esta_viva() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!Instance::new(port).is_alive());
    }
}
