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
use super::model::{Event, Failure, Job, Probe, Stage, State};
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
    pipeline: Arc<dyn Pipeline>,
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
    /// O job tem uma transcrição: removê-lo manda a pasta para a Lixeira, e isso precisa
    /// ser confirmado.
    NeedsConfirm,
    /// A pasta da saída não foi para a Lixeira; o job continua na lista.
    Trash,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RetryError {
    NotFound,
    /// Só um job que falhou volta para a fila.
    NotFailed,
}

impl Jobs {
    /// Cria a fila e o worker que a processa com `pipeline`. Os arquivos que sobraram em
    /// `uploads` de uma execução anterior são apagados: a fila nasce vazia.
    pub fn start(uploads: PathBuf, pipeline: Arc<dyn Pipeline>) -> io::Result<(Self, Worker)> {
        let (jobs, rx) = Self::create(uploads, pipeline.clone())?;
        let worker = Worker::spawn(jobs.clone(), rx, pipeline);
        Ok((jobs, worker))
    }

    /// Fila sem worker: os jobs ficam "Na fila" (testes da camada web). O pipeline só
    /// atende ao probe e ao rodapé.
    pub fn without_worker(uploads: PathBuf, pipeline: Arc<dyn Pipeline>) -> io::Result<Self> {
        Ok(Self::create(uploads, pipeline)?.0)
    }

    fn create(
        uploads: PathBuf,
        pipeline: Arc<dyn Pipeline>,
    ) -> io::Result<(Self, mpsc::UnboundedReceiver<Msg>)> {
        clear_uploads(&uploads)?;
        let (queue, rx) = mpsc::unbounded_channel();
        let inner = Inner {
            uploads,
            entries: RwLock::default(),
            queue,
            stopping: AtomicBool::new(false),
            pipeline,
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

    /// O probe do pipeline para um arquivo recém-gravado. Bloqueia.
    pub fn probe(&self, name: &str, input: &Path) -> Result<Probe, Failure> {
        self.0.pipeline.probe(name, input)
    }

    /// Onde a transcrição roda, para o rodapé.
    pub fn backend(&self) -> Option<String> {
        self.0.pipeline.backend()
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
        if job.state == State::Canceled {
            remove_quietly(&job.input);
        }
        Some(job)
    }

    /// "Tentar de novo": o job que falhou volta para a fila com o mesmo envio.
    pub fn retry(&self, id: JobId) -> Result<Job, RetryError> {
        let job = {
            let mut entries = self.write();
            let entry = entries.get_mut(&id).ok_or(RetryError::NotFound)?;
            entry.job.state = entry
                .job
                .state
                .apply(Event::Retry)
                .map_err(|_| RetryError::NotFailed)?;
            // Um pedido de cancelamento antigo não vale para a nova tentativa.
            entry.cancel = Arc::default();
            entry.job.clone()
        };
        let _ = self.0.queue.send(Msg::Run(id));
        Ok(job)
    }

    /// Tira um job terminado da lista e apaga o que sobrou dele em `uploads`. Num job
    /// concluído, só com `confirmed`: a pasta da saída vai para a Lixeira antes.
    pub fn remove(&self, id: JobId, confirmed: bool) -> Result<(), RemoveError> {
        let job = self.get(id).ok_or(RemoveError::NotFound)?;
        match &job.state {
            state if !state.is_terminal() => return Err(RemoveError::Active),
            State::Done { .. } if !confirmed => return Err(RemoveError::NeedsConfirm),
            // Apagada fora do app: não há o que mandar para a Lixeira.
            State::Done { outputs } if outputs.dir.exists() => {
                crate::trash::move_to_trash(&outputs.dir).map_err(|e| {
                    eprintln!(
                        "não consegui mandar {} para a Lixeira: {e}",
                        outputs.dir.display()
                    );
                    RemoveError::Trash
                })?;
            }
            _ => {}
        }
        if let Some(entry) = self.write().remove(&id) {
            remove_quietly(&entry.job.input);
        }
        Ok(())
    }

    /// Para o worker: tira o job da fila e devolve uma cópia dele e a flag de cancelamento;
    /// `None` se ele foi cancelado ou removido antes.
    pub(super) fn begin(&self, id: JobId) -> Option<(Job, Arc<AtomicBool>)> {
        let mut entries = self.write();
        let entry = entries.get_mut(&id)?;
        entry.job.state = entry
            .job
            .state
            .apply(Event::Start(Stage::ExtractingAudio))
            .ok()?;
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

    /// O fim do pipeline. O envio é apagado antes de publicar o fim (quem vê o job terminado
    /// já não acha o arquivo), exceto se o job falhou: ele fica para o "Tentar de novo".
    pub(super) fn finish(&self, id: JobId, event: Event) {
        let mut entries = self.write();
        let Some(entry) = entries.get_mut(&id) else {
            return;
        };
        let Ok(state) = entry.job.state.apply(event) else {
            return;
        };
        if !matches!(state, State::Failed { .. }) {
            remove_quietly(&entry.job.input);
        }
        entry.job.state = state;
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

fn remove_quietly(path: &Path) {
    let _ = fs::remove_file(path);
}
