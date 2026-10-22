//! O histórico: as pastas de `tests/fixtures/historico/` (o `job.json` de cada uma) voltam
//! como jobs concluídos na lista e no leitor, na ordem certa; as inválidas ficam de fora; o
//! limite de 50; e o áudio guardado volta como "Mostrar no Explorer".

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use scriba::capture::{Limits, Recorder, ScriptedSource};
use scriba::clock::LocalTime;
use scriba::config::Paths;
use scriba::dialog::ScriptedDialog;
use scriba::jobs::history::{self, LIMIT};
use scriba::jobs::{Audio, FakePipeline, Jobs, Media, State};
use scriba::opener::PrintOpener;
use scriba::transcript::{self, Header, Segment, Speaker, Transcript};
use scriba::web::{AppState, Desktop, router};
use tower::ServiceExt;

const TOKEN: &str = "segredo";
const PORT: u16 = 4321;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/historico")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Grava o `.txt` e o `.md` da pasta como o pipeline grava.
fn write_transcript(dir: &Path, recording: bool) {
    let mut segments = vec![Segment::new(0, 3_000, "Bom dia, pessoal.")];
    segments.push(Segment::new(5_000, 8_000, "Vamos ao orçamento."));
    if recording {
        segments[0].speaker = Some(Speaker::Voce);
        segments[1].speaker = Some(Speaker::Outros);
    }
    let header = Header {
        file: "x".into(),
        date: LocalTime::parse_rfc3339("2026-10-20T10:15:00-03:00").unwrap(),
        duration_ms: 8_000,
        recording,
    };
    let t = Transcript::new(header, &segments);
    fs::write(dir.join(transcript::TXT_FILE), transcript::render_txt(&t)).unwrap();
    fs::write(dir.join(transcript::MD_FILE), transcript::render_md(&t)).unwrap();
}

/// As fixtures copiadas para a pasta das saídas, com o texto das três válidas e o áudio
/// guardado da reunião.
fn setup(tmp: &tempfile::TempDir) -> (Paths, PathBuf) {
    let paths = Paths::resolve(
        Some(tmp.path().join("dados")),
        Some(tmp.path().join("saidas")),
    );
    copy_dir(&fixtures(), &paths.output_dir);
    let out = &paths.output_dir;
    write_transcript(&out.join("2026-10-18 0900 - Reunião de equipe"), false);
    write_transcript(&out.join("2026-10-19 1430 - Apresentação"), false);
    write_transcript(&out.join("2026-10-20 1015 - Gravação"), true);
    let audio = tmp.path().join("Áudios").join("Reunião de equipe.m4a");
    fs::create_dir_all(audio.parent().unwrap()).unwrap();
    fs::write(&audio, "áudio").unwrap();
    transcript::set_manifest_audio(&out.join("2026-10-18 0900 - Reunião de equipe"), &audio)
        .unwrap();
    (paths, audio)
}

fn state(paths: Paths, jobs: Jobs) -> AppState {
    let recorder = Recorder::new(
        Arc::new(ScriptedSource::new(&paths.data_dir)),
        paths.recordings_dir(),
        Limits::default(),
    );
    let dialog = Arc::new(ScriptedDialog::new(&paths.data_dir));
    AppState::new(
        TOKEN.into(),
        "inst-1".into(),
        PORT,
        paths,
        jobs,
        Desktop {
            opener: Arc::new(PrintOpener),
            dialog,
            recorder,
        },
    )
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let req = Request::get(uri)
        .header(header::HOST, format!("127.0.0.1:{PORT}"))
        .header(header::COOKIE, format!("scriba_token={TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let res = router(state.clone()).oneshot(req).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn transcricoes_voltam_concluidas_e_na_ordem() {
    let tmp = tempfile::tempdir().unwrap();
    let (paths, audio) = setup(&tmp);
    let pipeline = Arc::new(FakePipeline::new(paths.output_dir.clone()));
    let jobs = Jobs::without_worker(paths.uploads_dir(), pipeline).unwrap();
    scriba::app::restore_history(&jobs, &paths);

    let list = jobs.list();
    let names: Vec<&str> = list.iter().map(|j| j.name.as_str()).collect();
    // Quebrado, versão 2, sem job.json e o id repetido (a cópia, mais antiga) ficam de fora.
    assert_eq!(
        names,
        [
            "Gravação de 20/10, 10:15",
            "Apresentação.mp4",
            "Reunião de equipe.mp3",
            "Sem texto.wav"
        ]
    );
    let media: Vec<Media> = list.iter().map(|j| j.media).collect();
    assert_eq!(
        media,
        [Media::Recording, Media::Video, Media::Audio, Media::Audio]
    );
    assert!(!jobs.has_older());
    for job in &list {
        let State::Done { outputs, audio: a } = &job.state else {
            panic!("{job:?}");
        };
        assert!(outputs.dir.starts_with(&paths.output_dir));
        if job.name.starts_with("Reunião") {
            assert_eq!(
                a,
                &Audio::Saved {
                    path: audio.clone()
                }
            );
        } else {
            assert_eq!(a, &Audio::Discarded);
        }
    }
    assert_eq!(list[2].duration_ms, 1_800_000);
    assert_eq!(list[2].created.display(), "18/10/2026 09:00");

    let state = state(paths, jobs);
    let (status, home) = get(&state, "/").await;
    assert_eq!(status, StatusCode::OK);
    let pos = |name: &str| home.find(name).unwrap_or_else(|| panic!("{name}: {home}"));
    assert!(pos("Gravação de 20/10") < pos("Apresentação.mp4"));
    assert!(pos("Apresentação.mp4") < pos("Reunião de equipe.mp3"));
    assert!(home.contains("Mostrar no Explorer"), "o áudio guardado");
    assert!(!home.contains("Guardar o áudio?"), "nada a responder");
    assert!(!home.contains("As mais antigas"));

    // O leitor: a gravação com os rótulos, o arquivo sem.
    let (status, page) = get(&state, &format!("/jobs/{}", list[0].id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        page.contains("transcript-speaker") && page.contains("Outros"),
        "{page}"
    );
    let (_, page) = get(&state, &format!("/jobs/{}", list[1].id)).await;
    assert!(page.contains("Vamos ao orçamento.") && !page.contains("transcript-speaker"));
    // O .txt apagado: o leitor avisa.
    let (status, page) = get(&state, &format!("/jobs/{}", list[3].id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Não encontrei a transcrição"), "{page}");
}

/// Um áudio guardado que sumiu volta como descartado.
#[test]
fn audio_que_sumiu_volta_descartado() {
    let tmp = tempfile::tempdir().unwrap();
    let (paths, audio) = setup(&tmp);
    fs::remove_file(audio).unwrap();
    let h = history::load(&paths.output_dir, LIMIT);
    let reuniao = h
        .jobs
        .iter()
        .find(|j| j.name.starts_with("Reunião"))
        .unwrap();
    assert!(matches!(
        reuniao.state,
        State::Done {
            audio: Audio::Discarded,
            ..
        }
    ));
}

/// Com 60 pastas, a lista tem as 50 mais recentes e a linha das mais antigas.
#[tokio::test]
async fn limite_de_50() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(
        Some(tmp.path().join("dados")),
        Some(tmp.path().join("saidas")),
    );
    let model = fs::read_to_string(fixtures().join("2026-10-18 0900 - Reunião de equipe/job.json"))
        .unwrap();
    for n in 0..60u32 {
        let dir = paths.output_dir.join(format!("pasta {n:02}"));
        fs::create_dir_all(&dir).unwrap();
        // Dias diferentes de outubro e ids na mesma ordem.
        let json = model
            .replace(
                "01JA0000000000000000000002",
                &format!("01JA00000000000000000000{n:02}"),
            )
            .replace(
                "2026-10-18T09:00:00",
                &format!("2026-{:02}-{:02}T09:00:00", 8 + n / 30, 1 + n % 30),
            );
        fs::write(dir.join("job.json"), json).unwrap();
    }
    let pipeline = Arc::new(FakePipeline::new(paths.output_dir.clone()));
    let jobs = Jobs::without_worker(paths.uploads_dir(), pipeline).unwrap();
    scriba::app::restore_history(&jobs, &paths);
    let list = jobs.list();
    assert_eq!(list.len(), 50);
    assert!(jobs.has_older());
    let first = list.first().unwrap();
    let last = list.last().unwrap();
    assert_eq!(first.id.to_string(), "01JA0000000000000000000059");
    assert_eq!(last.id.to_string(), "01JA0000000000000000000010");

    let state = state(paths, jobs);
    let (_, home) = get(&state, "/").await;
    assert!(home.contains("As mais antigas continuam em"), "{home}");
    assert!(home.contains(r#"hx-post="/api/app/saidas""#));
}
