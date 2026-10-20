//! "Guardar o áudio?" no card concluído: Descartar, Guardar áudio (a janela "Salvar como" e
//! a gravação do `.m4a` numa thread da fila; o card acompanha pelo polling), cancelar a
//! gravação e "Mostrar no Explorer".

use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::AppState;
use super::jobs::{Card, fragment, not_found};
use super::reader::folder_label;
use crate::dialog::SaveRequest;
use crate::jobs::{Audio, AudioError, JobId, Outputs, State as JobState};

fn audio_error(e: AudioError) -> Response {
    let message = match e {
        AudioError::NotFound => return not_found(),
        AudioError::NotDone => "Espere a transcrição terminar.",
        AudioError::Busy => "Espere o áudio terminar de ser guardado.",
        AudioError::Answered => "O áudio deste job já foi guardado ou descartado.",
    };
    (StatusCode::CONFLICT, message).into_response()
}

/// `POST /api/jobs/{id}/audio/discard`: apaga o envio na hora; o card perde a pergunta.
pub async fn discard(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<JobId>() else {
        return not_found();
    };
    match state.jobs.discard_audio(id) {
        Ok(job) => {
            let notice = format!("Áudio de {} descartado.", job.name);
            fragment(StatusCode::OK, vec![Card::new(&job)], Some(notice))
        }
        Err(e) => audio_error(e),
    }
}

/// `POST /api/jobs/{id}/audio/save`: responde na hora com o card em "Escolhendo onde
/// guardar…", que acompanha pelo polling a janela e a gravação, feitas numa thread da fila.
/// A pasta escolhida fica no `config.json` para a próxima vez.
pub async fn save(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<JobId>() else {
        return not_found();
    };
    let app = state.clone();
    let ask = move |_: &_, outputs: &Outputs| {
        let request = SaveRequest {
            file_name: suggested_name(outputs),
            dir: initial_dir(
                app.config().audio_dir.as_deref(),
                &app.paths.output_dir,
                &outputs.dir,
            ),
        };
        let chosen = app.dialog.ask(&request)?;
        if let Some(dir) = chosen.as_deref().and_then(FsPath::parent) {
            app.update_config(|c| c.audio_dir = Some(dir.to_path_buf()));
        }
        Ok(chosen)
    };
    match state.jobs.save_audio(id, ask) {
        Ok(job) => fragment(StatusCode::OK, vec![Card::new(&job)], None),
        Err(e) => audio_error(e),
    }
}

/// `POST /api/jobs/{id}/audio/cancel`: pede o fim da gravação; o polling mostra a pergunta
/// de volta. Fora da gravação, devolve o card como está.
pub async fn cancel(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(job) = id
        .parse::<JobId>()
        .ok()
        .and_then(|id| state.jobs.cancel_audio(id))
    else {
        return not_found();
    };
    fragment(StatusCode::OK, vec![Card::new(&job)], None)
}

/// `POST /api/jobs/{id}/audio/show`: a pasta do áudio guardado no Explorer, com ele
/// selecionado. 204 quando abre.
pub async fn show(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(job) = id.parse::<JobId>().ok().and_then(|id| state.jobs.get(id)) else {
        return not_found();
    };
    let JobState::Done {
        audio: Audio::Saved { path },
        ..
    } = job.state
    else {
        return (StatusCode::CONFLICT, "O áudio deste job não foi guardado.").into_response();
    };
    if !path.is_file() {
        return (
            StatusCode::NOT_FOUND,
            format!(
                "Não encontrei o áudio “{}”. Ele foi movido ou apagado.",
                folder_label(&path)
            ),
        )
            .into_response();
    }
    let opener = state.opener.clone();
    let file = path.clone();
    match tokio::task::spawn_blocking(move || opener.reveal(&file)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        result => {
            if let Ok(Err(e)) = result {
                eprintln!("não consegui mostrar {}: {e}", path.display());
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Não consegui abrir a pasta.",
            )
                .into_response()
        }
    }
}

/// O nome sugerido segue a pasta da transcrição: `2026-10-20 1430 - Reunião.m4a`.
pub fn suggested_name(outputs: &Outputs) -> String {
    let folder = folder_label(&outputs.dir);
    let stem = if folder.is_empty() { "Áudio" } else { &folder };
    format!("{stem}.m4a")
}

/// Onde a janela abre: a última pasta escolhida, se ela ainda existe e não é a pasta de
/// outra transcrição (filha de `output_dir`); senão a pasta desta transcrição (ou, se ela
/// sumiu, a das saídas).
pub fn initial_dir(last: Option<&FsPath>, output_dir: &FsPath, job_dir: &FsPath) -> PathBuf {
    match last {
        Some(dir) if dir.is_dir() && dir.parent() != Some(output_dir) => dir.to_path_buf(),
        _ if job_dir.is_dir() => job_dir.to_path_buf(),
        _ => output_dir.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nome_sugerido_segue_a_pasta() {
        let outputs = |dir: &str| Outputs {
            dir: dir.into(),
            ..Outputs::default()
        };
        assert_eq!(
            suggested_name(&outputs(r"C:\Saidas\2026-10-20 1430 - Reunião")),
            "2026-10-20 1430 - Reunião.m4a"
        );
        assert_eq!(suggested_name(&outputs("")), "Áudio.m4a");
    }

    #[test]
    fn pasta_inicial() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("Scriba");
        let job = output.join("2026-10-20 1430 - Reunião");
        let other_job = output.join("2026-10-19 0900 - Aula");
        let music = root.path().join("Música");
        for dir in [&job, &other_job, &music] {
            std::fs::create_dir_all(dir).unwrap();
        }
        // Na 1ª vez, a pasta desta transcrição.
        assert_eq!(initial_dir(None, &output, &job), job);
        // Depois, a última escolhida.
        assert_eq!(initial_dir(Some(&music), &output, &job), music);
        assert_eq!(initial_dir(Some(&output), &output, &job), output);
        // A última foi a pasta de outra transcrição: esta.
        assert_eq!(initial_dir(Some(&other_job), &output, &job), job);
        // A última sumiu: esta.
        assert_eq!(
            initial_dir(Some(&root.path().join("sumiu")), &output, &job),
            job
        );
        // Esta também sumiu: a das saídas.
        std::fs::remove_dir(&job).unwrap();
        assert_eq!(initial_dir(None, &output, &job), output);
    }
}
