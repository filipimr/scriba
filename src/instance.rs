//! Instância única: a instância em execução se registra em `instance.json`; uma segunda
//! execução confere se ela responde e, nesse caso, só reabre o navegador nela.

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

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
    fn porta_sem_servidor_nao_esta_viva() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!Instance::new(port).is_alive());
    }
}
