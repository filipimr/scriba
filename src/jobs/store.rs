//! Os jobs em memória, em ordem de criação, e a fila do worker. Até a Fase 7 nada é
//! persistido: um reinício começa com a lista vazia.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use tokio::sync::mpsc;

use super::id::JobId;
use super::model::{Event, Job, State};
use super::pipeline::Pipeline;
use super::worker::Worker;

/// Mensagens para o worker.
pub(super) enum Msg {
    Run(JobId),
    Stop,
}

/// Handle clonável da fila: o servidor envia e consulta; o worker atualiza.
#[derive(Clone)]
pub struct Jobs(Arc<Inner>);

struct Inner {
    uploads: PathBuf,
    entries: RwLock<BTreeMap<JobId, Entry>>,
    queue: mpsc::UnboundedSender<Msg>,
    stopping: AtomicBool,
}

struct Entry {
    job: Job,
    /// Lido pelo pipeline sem pegar o lock.
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RemoveError {
    NotFound,
    /// Job na fila ou rodando: precisa ser cancelado antes.
    Active,
}

impl Jobs {
    /// Cria a fila e o worker que a processa com `pipeline`. Os arquivos que sobraram em
    /// `uploads` de uma execução anterior são apagados: a fila nasce vazia.
    pub fn start(uploads: PathBuf, pipeline: Arc<dyn Pipeline>) -> io::Result<(Self, Worker)> {
        let (jobs, rx) = Self::create(uploads)?;
        let worker = Worker::spawn(jobs.clone(), rx, pipeline);
        Ok((jobs, worker))
    }

    /// Fila sem worker: os jobs ficam "Na fila" (testes da camada web).
    pub fn without_worker(uploads: PathBuf) -> io::Result<Self> {
        Ok(Self::create(uploads)?.0)
    }

    fn create(uploads: PathBuf) -> io::Result<(Self, mpsc::UnboundedReceiver<Msg>)> {
        clear_uploads(&uploads)?;
        let (queue, rx) = mpsc::unbounded_channel();
        let inner = Inner {
            uploads,
            entries: RwLock::default(),
            queue,
            stopping: AtomicBool::new(false),
        };
        Ok((Self(Arc::new(inner)), rx))
    }

    pub fn uploads_dir(&self) -> &Path {
        &self.0.uploads
    }

    /// Onde gravar o arquivo enviado: o nome vem do id, nunca do cliente.
    pub fn upload_path(&self, id: JobId, ext: &str) -> PathBuf {
        self.0.uploads.join(format!("{id}.{ext}"))
    }

    /// Põe o job (já com o arquivo gravado) na fila.
    pub fn submit(&self, job: Job) {
        let id = job.id;
        let entry = Entry {
            job,
            cancel: Arc::default(),
        };
        self.write().insert(id, entry);
        // Sem worker (testes) ou encerrando: o job fica na fila.
        let _ = self.0.queue.send(Msg::Run(id));
    }

    /// Do mais recente para o mais antigo.
    pub fn list(&self) -> Vec<Job> {
        self.read().values().rev().map(|e| e.job.clone()).collect()
    }

    pub fn get(&self, id: JobId) -> Option<Job> {
        self.read().get(&id).map(|e| e.job.clone())
    }

    /// Pede o cancelamento. Na fila, o job termina na hora; rodando, o pipeline vê a flag
    /// e o worker registra o fim. Em job já terminado, não muda nada.
    pub fn cancel(&self, id: JobId) -> Option<Job> {
        let job = {
            let mut entries = self.write();
            let entry = entries.get_mut(&id)?;
            entry.cancel.store(true, Ordering::Relaxed);
            if let Ok(state) = entry.job.state.apply(Event::Cancel) {
                entry.job.state = state;
            }
            entry.job.clone()
        };
        if job.state.is_terminal() {
            remove_quietly(&job.input);
        }
        Some(job)
    }

    /// Tira um job terminado da lista e apaga o que sobrou dele em `uploads`.
    pub fn remove(&self, id: JobId) -> Result<(), RemoveError> {
        let mut entries = self.write();
        match entries.get(&id) {
            None => return Err(RemoveError::NotFound),
            Some(e) if !e.job.state.is_terminal() => return Err(RemoveError::Active),
            Some(_) => {}
        }
        if let Some(entry) = entries.remove(&id) {
            drop(entries);
            remove_quietly(&entry.job.input);
        }
        Ok(())
    }

    /// Para o worker: tira o job da fila e devolve uma cópia dele e a flag de cancelamento;
    /// `None` se ele foi cancelado ou removido antes.
    pub(super) fn begin(&self, id: JobId) -> Option<(Job, Arc<AtomicBool>)> {
        let mut entries = self.write();
        let entry = entries.get_mut(&id)?;
        let first = *entry.job.options.stages().first()?;
        entry.job.state = entry.job.state.apply(Event::Start(first)).ok()?;
        Some((entry.job.clone(), entry.cancel.clone()))
    }

    /// Aplica um evento do worker; transições inválidas são ignoradas.
    pub(super) fn apply(&self, id: JobId, event: Event) {
        if let Some(entry) = self.write().get_mut(&id)
            && let Ok(state) = entry.job.state.apply(event)
        {
            entry.job.state = state;
        }
    }

    /// Encerramento: cancela o job em andamento e impede que outros comecem.
    pub(super) fn stop(&self) {
        self.0.stopping.store(true, Ordering::Relaxed);
        for entry in self.write().values_mut() {
            if matches!(entry.job.state, State::Running { .. }) {
                entry.cancel.store(true, Ordering::Relaxed);
                if let Ok(state) = entry.job.state.apply(Event::Cancel) {
                    entry.job.state = state;
                }
            }
        }
        // Acorda o worker se ele estiver esperando a fila.
        let _ = self.0.queue.send(Msg::Stop);
    }

    pub(super) fn stopping(&self) -> bool {
        self.0.stopping.load(Ordering::Relaxed)
    }

    fn read(&self) -> RwLockReadGuard<'_, BTreeMap<JobId, Entry>> {
        self.0
            .entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, BTreeMap<JobId, Entry>> {
        self.0
            .entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Esvazia `uploads` (criando a pasta se preciso). Arquivo preso por outro programa fica
/// para a próxima vez.
fn clear_uploads(dir: &Path) -> io::Result<()> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            remove_quietly(&entry.path());
        }
    }
    fs::create_dir_all(dir)
}

pub(super) fn remove_quietly(path: &Path) {
    let _ = fs::remove_file(path);
}
