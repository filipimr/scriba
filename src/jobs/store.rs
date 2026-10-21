//! Os jobs em memória, em ordem de criação, e a fila do worker. Até a Fase 7 nada é
//! persistido: um reinício começa com a lista vazia (as gravações ainda não transcritas
//! voltam pelo `capture::recover`).
//!
//! A mídia de um job (o envio em `uploads/` ou a pasta da gravação) sai ao cancelar, ao
//! remover e com a resposta a "Guardar o áudio?". Ao fechar o app, os envios saem todos; a
//! gravação, só a já transcrita: a que estava na fila, rodando ou que falhou volta na
//! próxima abertura.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::{self, JoinHandle};

use tokio::sync::mpsc;

use super::id::JobId;
use super::model::{
    Audio, AudioEvent, Event, Failure, Job, Media, Outputs, PANIC_MESSAGE, Probe, Stage, State,
};
use super::pipeline::Pipeline;
use super::worker::Worker;
use crate::capture::{Recording, storage};

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
    /// A gravação do áudio ("Guardar áudio"), numa thread própria, e a flag que a cancela.
    audio: Option<(JoinHandle<()>, Arc<AtomicBool>)>,
}

/// Por que a pergunta "Guardar o áudio?" não aceitou a resposta.
#[derive(Debug, PartialEq, Eq)]
pub enum AudioError {
    NotFound,
    /// O job ainda não concluiu (ou falhou, ou foi cancelado).
    NotDone,
    /// A janela "Salvar como" está aberta ou o áudio está sendo gravado.
    Busy,
    /// O áudio já foi guardado ou descartado.
    Answered,
}

/// Mensagem da janela "Salvar como" que não abriu.
pub const DIALOG_ERROR: &str = "Não consegui abrir a janela Salvar como. Tente de novo.";

#[derive(Debug, PartialEq, Eq)]
pub enum RemoveError {
    NotFound,
    /// Job na fila ou rodando: precisa ser cancelado antes.
    Active,
    /// O áudio está sendo guardado (ou a janela "Salvar como" está aberta).
    AudioBusy,
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
            audio: None,
        };
        self.write().insert(id, entry);
        // Sem worker (testes) ou encerrando: o job fica na fila.
        let _ = self.0.queue.send(Msg::Run(id));
    }

    /// Põe uma gravação terminada na fila, com a estimativa do pipeline.
    pub fn submit_recording(&self, rec: Recording) -> JobId {
        let estimate = self.0.pipeline.estimate(rec.duration_ms);
        let job = Job::recording(rec, estimate);
        let id = job.id;
        self.submit(job);
        id
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
            discard_input(&job);
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

    /// Tira um job terminado da lista e apaga a mídia que sobrou dele. Num job
    /// concluído, só com `confirmed`: a pasta da saída vai para a Lixeira antes.
    pub fn remove(&self, id: JobId, confirmed: bool) -> Result<(), RemoveError> {
        let job = self.get(id).ok_or(RemoveError::NotFound)?;
        match &job.state {
            state if !state.is_terminal() => return Err(RemoveError::Active),
            State::Done { audio, .. } if audio.busy() => return Err(RemoveError::AudioBusy),
            State::Done { .. } if !confirmed => return Err(RemoveError::NeedsConfirm),
            // Apagada fora do app: não há o que mandar para a Lixeira.
            State::Done { outputs, .. } if outputs.dir.exists() => {
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
            discard_input(&entry.job);
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

    /// O fim do pipeline. Um job cancelado tem o envio apagado antes de publicar o fim (quem
    /// vê o job terminado já não acha o arquivo); o concluído o guarda até a resposta a
    /// "Guardar o áudio?", e o que falhou, para o "Tentar de novo". Uma gravação cancelada
    /// pelo encerramento (Ctrl+C no meio da transcrição) fica, para voltar na próxima
    /// abertura; a concluída é marcada como transcrita no `gravacao.json`.
    pub(super) fn finish(&self, id: JobId, event: Event) {
        let mut entries = self.write();
        let Some(entry) = entries.get_mut(&id) else {
            return;
        };
        let Ok(state) = entry.job.state.apply(event) else {
            return;
        };
        let recording = entry.job.media == Media::Recording;
        match &state {
            State::Canceled if !(recording && self.stopping()) => discard_input(&entry.job),
            State::Done { .. } if recording => {
                if let Err(e) = storage::mark_transcribed(&entry.job.input) {
                    eprintln!(
                        "não marquei a gravação {} como transcrita: {e}",
                        entry.job.input.display()
                    );
                }
            }
            _ => {}
        }
        entry.job.state = state;
    }

    /// "Descartar": o envio do job concluído é apagado na hora.
    pub fn discard_audio(&self, id: JobId) -> Result<Job, AudioError> {
        let job = self.answer(id, AudioEvent::Discard)?;
        discard_input(&job);
        Ok(job)
    }

    /// "Guardar áudio": numa thread própria, `ask` abre a janela "Salvar como" (devolve o
    /// arquivo escolhido, ou `None` se o usuário cancelou) e o pipeline grava o `.m4a` em
    /// `<arquivo>.parte`, renomeado no fim. Com sucesso, o envio é apagado; cancelada ou com
    /// erro, a pergunta volta e o envio fica. Devolve o job já com a janela aberta.
    pub fn save_audio(
        &self,
        id: JobId,
        ask: impl FnOnce(&Job, &Outputs) -> io::Result<Option<PathBuf>> + Send + 'static,
    ) -> Result<Job, AudioError> {
        let job = self.answer(id, AudioEvent::Choose)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let thread = {
            let (jobs, job, cancel) = (self.clone(), job.clone(), cancel.clone());
            thread::Builder::new()
                .name(format!("scriba-audio-{id}"))
                .spawn(move || jobs.run_save(&job, ask, &cancel))
                .expect("criando a thread do áudio")
        };
        if let Some(entry) = self.write().get_mut(&id) {
            entry.audio = Some((thread, cancel));
        }
        Ok(job)
    }

    /// Pede o cancelamento da gravação do áudio; fora dela, não muda nada.
    pub fn cancel_audio(&self, id: JobId) -> Option<Job> {
        let mut entries = self.write();
        let entry = entries.get_mut(&id)?;
        if let State::Done {
            audio: Audio::Saving { .. },
            ..
        } = entry.job.state
            && let Some((_, cancel)) = &entry.audio
        {
            cancel.store(true, Ordering::Relaxed);
            if let Ok(state) = entry.job.state.apply(Event::Audio(AudioEvent::Cancel)) {
                entry.job.state = state;
            }
        }
        Some(entry.job.clone())
    }

    /// Aplica a resposta do usuário e devolve o job; um erro diz por que ela não vale.
    fn answer(&self, id: JobId, event: AudioEvent) -> Result<Job, AudioError> {
        let mut entries = self.write();
        let entry = entries.get_mut(&id).ok_or(AudioError::NotFound)?;
        let State::Done { audio, .. } = &entry.job.state else {
            return Err(AudioError::NotDone);
        };
        let error = if audio.busy() {
            AudioError::Busy
        } else {
            AudioError::Answered
        };
        entry.job.state = entry
            .job
            .state
            .apply(Event::Audio(event))
            .map_err(|_| error)?;
        Ok(entry.job.clone())
    }

    /// A thread do "Guardar áudio". Um panic (bug) vira a mensagem genérica de erro.
    fn run_save(
        &self,
        job: &Job,
        ask: impl FnOnce(&Job, &Outputs) -> io::Result<Option<PathBuf>>,
        cancel: &AtomicBool,
    ) {
        let State::Done { outputs, .. } = &job.state else {
            return;
        };
        let id = job.id;
        let chosen = match panic::catch_unwind(AssertUnwindSafe(|| ask(job, outputs))) {
            Ok(Ok(chosen)) => chosen,
            outcome => {
                if let Ok(Err(e)) = outcome {
                    eprintln!("a janela Salvar como não abriu: {e}");
                }
                let error = Failure::Error(DIALOG_ERROR.into());
                self.audio(id, AudioEvent::Finished(Err(error)));
                return;
            }
        };
        // Escolhido depois do Ctrl+C: o envio já vai ser apagado.
        let Some(path) = chosen.filter(|_| !self.stopping()) else {
            self.audio(id, AudioEvent::Chosen(None));
            return;
        };
        self.audio(id, AudioEvent::Chosen(Some(path.clone())));
        let part = part_path(&path);
        let mut progress = |pct| self.audio(id, AudioEvent::Progress(pct));
        let saved = panic::catch_unwind(AssertUnwindSafe(|| {
            self.0
                .pipeline
                .save_audio(job, &part, &mut progress, cancel)
        }))
        .unwrap_or_else(|_| Err(Failure::Error(PANIC_MESSAGE.into())))
        .and_then(|()| {
            // O "Salvar como" já perguntou se pode substituir um arquivo que existe.
            fs::rename(&part, &path).map_err(|e| {
                eprintln!("não consegui renomear {}: {e}", part.display());
                crate::media::Error::Encode.into()
            })
        });
        if saved.is_err() {
            remove_quietly(&part);
        }
        // Com o áudio guardado, o envio é apagado antes de publicar o fim.
        let mut entries = self.write();
        let Some(entry) = entries.get_mut(&id) else {
            return;
        };
        let event = Event::Audio(AudioEvent::Finished(saved.map(|()| path)));
        if let Ok(state) = entry.job.state.apply(event) {
            if let State::Done {
                audio: Audio::Saved { .. },
                ..
            } = state
            {
                discard_input(&entry.job);
            }
            entry.job.state = state;
        }
    }

    /// Aplica um evento da gravação do áudio; transições inválidas são ignoradas.
    fn audio(&self, id: JobId, event: AudioEvent) {
        self.apply(id, Event::Audio(event));
    }

    /// Encerramento: cancela o job em andamento e as gravações de áudio, e impede que outros
    /// comecem.
    pub(super) fn stop(&self) {
        self.0.stopping.store(true, Ordering::Relaxed);
        for entry in self.write().values_mut() {
            let event = match &entry.job.state {
                State::Running { .. } => {
                    entry.cancel.store(true, Ordering::Relaxed);
                    Event::Cancel
                }
                State::Done {
                    audio: Audio::Saving { .. },
                    ..
                } => {
                    if let Some((_, cancel)) = &entry.audio {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    Event::Audio(AudioEvent::Cancel)
                }
                _ => continue,
            };
            if let Ok(state) = entry.job.state.apply(event) {
                entry.job.state = state;
            }
        }
        // Acorda o worker se ele estiver esperando a fila.
        let _ = self.0.queue.send(Msg::Stop);
    }

    /// As threads que gravam áudio agora, para o encerramento esperar. As que estão com a
    /// janela aberta ficam de fora: a janela só fecha com o usuário, e o processo sai mesmo
    /// assim.
    pub(super) fn saving_threads(&self) -> Vec<JoinHandle<()>> {
        self.write()
            .values_mut()
            .filter(|e| {
                matches!(
                    e.job.state,
                    State::Done {
                        audio: Audio::Saving { .. },
                        ..
                    }
                )
            })
            .filter_map(|e| e.audio.take().map(|(thread, _)| thread))
            .collect()
    }

    /// Encerramento: os envios saem todos, e as gravações já transcritas que esperavam a
    /// resposta também. As outras gravações ficam e voltam na próxima abertura.
    pub(super) fn clear_media(&self) {
        if let Err(e) = clear_uploads(&self.0.uploads) {
            eprintln!("não consegui esvaziar {}: {e}", self.0.uploads.display());
        }
        for entry in self.read().values() {
            if let State::Done { audio, .. } = &entry.job.state
                && entry.job.media == Media::Recording
                && !audio.answered()
            {
                discard_input(&entry.job);
            }
        }
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

/// Apaga a mídia do job: o envio, ou a pasta inteira da gravação.
fn discard_input(job: &Job) {
    match job.media {
        Media::Recording => storage::remove_dir_quietly(&job.input),
        Media::Video | Media::Audio => remove_quietly(&job.input),
    }
}

/// Onde o `.m4a` é gravado antes de ganhar o nome escolhido: `<arquivo>.parte`, na mesma
/// pasta (o renome no fim não copia nada). Um cancelamento ou erro o apaga.
pub fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map_or_else(OsString::new, OsString::from);
    name.push(".parte");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parte_fica_ao_lado_do_escolhido() {
        assert_eq!(
            part_path(Path::new(r"C:\Áudios\Reunião.m4a")),
            PathBuf::from(r"C:\Áudios\Reunião.m4a.parte")
        );
    }
}
