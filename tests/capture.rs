//! A gravação ao vivo pelo `Recorder`: com o `FakeSource` (CI), que simula lacunas, jitter,
//! deriva do relógio e erros de dispositivo; e, na suíte de hardware (`#[ignore]`), com o
//! microfone e o loopback reais desta máquina.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scriba::capture::align::RATE;
use scriba::capture::fake::{Script, Signal, TrackScript};
use scriba::capture::{
    AudioSource, Choice, End, Error, EventKind, FakeSource, Limits, Recorder, Recording, Status,
    Track, recover, storage,
};

/// O relógio da fonte falsa corre 5× mais rápido: 30 s de gravação em 6 s.
const SPEED: f64 = 5.0;
/// 10 ms a 32 kHz.
const TEN_MS: usize = (RATE / 100) as usize;

type Done = Arc<Mutex<Vec<Recording>>>;

fn recorder(source: Arc<FakeSource>, limits: Limits) -> (Recorder, tempfile::TempDir, Done) {
    let dir = tempfile::tempdir().unwrap();
    let recorder = Recorder::new(source, dir.path().join("gravacoes"), limits);
    let done: Done = Arc::default();
    let sink = done.clone();
    recorder.on_finish(move |r| sink.lock().unwrap().push(r));
    (recorder, dir, done)
}

fn impulses() -> Signal {
    Signal::Impulses {
        every_ms: 1_000,
        amp: 0.9,
    }
}

fn now_ms(source: &FakeSource) -> u64 {
    source.now_ns() / 1_000_000
}

/// Espera `cond` por até `secs` segundos de verdade.
fn wait(secs: u64, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cond()
}

fn elapsed(recorder: &Recorder) -> u64 {
    match recorder.status() {
        Status::Recording { elapsed_ms, .. } => elapsed_ms,
        Status::Idle { .. } => u64::MAX,
    }
}

fn idle(recorder: &Recorder) -> bool {
    matches!(recorder.status(), Status::Idle { .. })
}

fn wav(dir: &Path, track: Track) -> Vec<f32> {
    let mut reader = hound::WavReader::open(dir.join(track.file())).unwrap();
    let spec = reader.spec();
    assert_eq!(
        (spec.sample_rate, spec.channels, spec.bits_per_sample),
        (RATE, 1, 16)
    );
    reader
        .samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32_768.0)
        .collect()
}

/// As posições dos picos acima de `threshold`, um por grupo de 10 ms.
fn peaks(samples: &[f32], threshold: f32) -> Vec<usize> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < samples.len() {
        if samples[i].abs() > threshold {
            let end = (i + TEN_MS).min(samples.len());
            let top = (i..end)
                .max_by(|&a, &b| samples[a].abs().total_cmp(&samples[b].abs()))
                .unwrap();
            found.push(top);
            i = end;
        } else {
            i += 1;
        }
    }
    found
}

fn rms(samples: &[f32]) -> f32 {
    scriba::capture::align::rms(samples)
}

/// Impulsos nos mesmos instantes nas duas trilhas, com o microfone a 44,1 kHz derivando
/// +500 ppm, a saída a 48 kHz estéreo com 15 s sem pacotes (o loopback mudo), e jitter na
/// entrega e no timestamp: os picos ficam a ±10 ms entre as trilhas e do segundo certo.
#[test]
fn trilhas_alinhadas_com_lacunas_jitter_e_deriva() {
    let mut script = Script::tones();
    script.mic = TrackScript {
        ppm: 500,
        delivery_jitter_ms: 20,
        timestamp_jitter_us: 1_000,
        ..TrackScript::new("Microfone", 44_100, 1, impulses())
    };
    script.system = TrackScript {
        delivery_jitter_ms: 20,
        timestamp_jitter_us: 1_000,
        packet_ms: 7,
        ..TrackScript::new("Saída", 48_000, 2, impulses())
    };
    let source = Arc::new(FakeSource::with_speed(script, SPEED));
    let t = now_ms(&source);
    let gap = t + 5_000..t + 20_000;
    source.edit(|s| s.system.gaps = vec![gap]);
    let (rec, _dir, done) = recorder(source, Limits::default());
    rec.start(&Choice::default()).unwrap();
    assert!(wait(60, || elapsed(&rec) >= 30_000));
    let r = rec.stop().unwrap();
    assert_eq!(r.end, End::User);
    assert_eq!(done.lock().unwrap().len(), 1);

    let (mic, sys) = (wav(&r.dir, Track::Mic), wav(&r.dir, Track::System));
    assert_eq!(mic.len(), sys.len(), "as trilhas terminam juntas");
    assert!(mic.len() as u64 >= 30 * u64::from(RATE));
    // Os pacotes do último ~1 s podem não ter chegado quando o Parar fechou os streams (o
    // relógio acelerado aumenta isso numa máquina ocupada): ficam de fora.
    let until = mic.len() - 2 * RATE as usize;
    let (pm, ps) = (peaks(&mic[..until], 0.2), peaks(&sys[..until], 0.2));
    assert!(pm.len() >= 27, "{} picos no microfone", pm.len());
    for w in pm.windows(2) {
        let gap = w[1] - w[0];
        assert!(
            gap.abs_diff(RATE as usize) <= TEN_MS,
            "picos do microfone a {gap} amostras"
        );
    }
    // A lacuna de 15 s some da saída; o resto fica casado com o microfone.
    assert!(
        ps.len() + 16 >= pm.len() && ps.len() + 14 <= pm.len(),
        "{} picos na saída, {} no microfone",
        ps.len(),
        pm.len()
    );
    for p in &ps {
        let nearest = pm.iter().map(|m| m.abs_diff(*p)).min().unwrap();
        assert!(
            nearest <= TEN_MS,
            "pico da saída em {p}: {nearest} amostras do microfone"
        );
    }
    let worst = ps
        .iter()
        .map(|p| pm.iter().map(|m| m.abs_diff(*p)).min().unwrap())
        .max()
        .unwrap();
    let [mic_stats, sys_stats] = r.stats;
    println!(
        "picos: {} no microfone, {} na saída, pior diferença {:.2} ms\nmicrofone: {mic_stats:?}\nsaída: {sys_stats:?}",
        pm.len(),
        ps.len(),
        worst as f64 / 32.0
    );
    assert!(sys_stats.filled >= 14 * u64::from(RATE), "{sys_stats:?}");
    assert!(mic_stats.max_error <= TEN_MS as u64, "{mic_stats:?}");
    assert_eq!((mic_stats.lost, sys_stats.lost), (0, 0));
}

#[test]
fn microfone_perdido_finaliza_os_arquivos() {
    let source = Arc::new(FakeSource::with_speed(Script::tones(), SPEED));
    let t = now_ms(&source);
    source.edit(|s| s.mic.fail = Some((t + 3_000, EventKind::Lost)));
    let (rec, _dir, done) = recorder(source, Limits::default());
    rec.start(&Choice::default()).unwrap();
    assert!(wait(60, || idle(&rec)));

    let done = done.lock().unwrap();
    assert_eq!(done.len(), 1, "on_finish uma vez");
    let r = &done[0];
    assert_eq!(r.end, End::MicLost);
    assert_eq!(
        rec.status(),
        Status::Idle {
            last: Some((r.id, End::MicLost))
        }
    );
    let (mic, sys) = (wav(&r.dir, Track::Mic), wav(&r.dir, Track::System));
    assert_eq!(mic.len(), sys.len());
    // O tom do microfone vai até o erro, aos 3 s do relógio (o Gravar veio logo depois de
    // marcar o relógio); dali ao fim, só o silêncio que completa a trilha.
    let last = mic.iter().rposition(|s| s.abs() > 0.01).unwrap();
    let secs = last as f64 / f64::from(RATE);
    assert!(
        (2.5..3.2).contains(&secs),
        "último som do microfone em {secs} s"
    );
    assert!(rms(&sys[..last]) > 0.05, "a saída seguiu gravando");
    assert_eq!(
        storage::read_manifest(&r.dir).unwrap().state,
        storage::RecState::Parada
    );
    assert_eq!(rec.stop(), Err(Error::NotRecording));
}

/// O computador dormiu no intervalo da reunião: 60 s sem pacotes nas duas trilhas. Ao
/// acordar, a gravação segue, sem o minuto suspenso (na 1ª noite do teste de 1 h, cada
/// suspensão virava horas de silêncio) e com as trilhas ainda alinhadas.
#[test]
fn suspensao_some_da_gravacao_e_ela_segue() {
    let mut script = Script::tones();
    script.mic = TrackScript::new("Microfone", 44_100, 1, impulses());
    script.system = TrackScript::new("Saída", 48_000, 2, impulses());
    let source = Arc::new(FakeSource::with_speed(script, 10.0));
    let t = now_ms(&source);
    source.edit(|s| {
        let sleep = t + 3_000..t + 63_000;
        s.mic.gaps = vec![sleep.clone()];
        s.system.gaps = vec![sleep];
    });
    let (rec, _dir, _done) = recorder(source, Limits::default());
    rec.start(&Choice::default()).unwrap();
    // Depois de acordar, o decorrido conta só o tempo gravado. (Aqui o processo segue
    // rodando durante a "suspensão"; num computador suspenso, a thread nem roda.)
    let suspended = || {
        matches!(
            rec.status(),
            Status::Recording {
                suspended: true,
                ..
            }
        )
    };
    assert!(wait(60, suspended));
    assert!(wait(60, || elapsed(&rec) >= 8_000));
    let r = rec.stop().unwrap();
    assert_eq!(r.end, End::User);
    let (mic, sys) = (wav(&r.dir, Track::Mic), wav(&r.dir, Track::System));
    // ~8 s gravados; numa máquina ocupada o Parar chega um pouco depois, mas o minuto
    // suspenso nunca entra.
    let secs = mic.len() as f64 / f64::from(RATE);
    assert!((8.0..30.0).contains(&secs), "{secs} s");
    let suspended = r.stats.iter().map(|s| s.suspended).max().unwrap();
    assert!(suspended >= 55 * u64::from(RATE), "{:?}", r.stats);
    // Depois de acordar, os picos das duas trilhas continuam casados.
    let until = mic.len() - RATE as usize;
    let (pm, ps) = (peaks(&mic[..until], 0.2), peaks(&sys[..until], 0.2));
    assert!(pm.len() >= 6, "{} picos", pm.len());
    for p in &ps {
        let nearest = pm.iter().map(|m| m.abs_diff(*p)).min().unwrap();
        assert!(
            nearest <= TEN_MS,
            "pico da saída em {p}: {nearest} amostras"
        );
    }
}

/// Um dispositivo volta de uma suspensão com o relógio 1 h à frente: só a trilha dele é
/// reancorada, e a outra não perde nada.
#[test]
fn timestamp_no_futuro_reancora_so_a_trilha() {
    let mut script = Script::tones();
    script.mic = TrackScript::new("Microfone", 48_000, 1, impulses());
    script.system = TrackScript::new("Saída", 48_000, 2, impulses());
    let source = Arc::new(FakeSource::with_speed(script, SPEED));
    let t = now_ms(&source);
    source.edit(|s| s.mic.clock_jump = Some((t + 3_000, 3_600_000)));
    let (rec, _dir, _done) = recorder(source, Limits::default());
    rec.start(&Choice::default()).unwrap();
    assert!(wait(60, || elapsed(&rec) >= 8_000));
    let r = rec.stop().unwrap();
    assert_eq!(r.stats[Track::Mic.index()].rebases, 1, "{:?}", r.stats);
    assert_eq!(r.stats[Track::System.index()].rebases, 0);
    let (mic, sys) = (wav(&r.dir, Track::Mic), wav(&r.dir, Track::System));
    let secs = mic.len() as f64 / f64::from(RATE);
    assert!(secs < 12.0, "{secs} s: a hora do salto virou silêncio");
    let until = mic.len() - RATE as usize;
    let (pm, ps) = (peaks(&mic[..until], 0.2), peaks(&sys[..until], 0.2));
    assert!(ps.len() >= 6 && pm.len() >= 6);
    for p in &ps {
        let nearest = pm.iter().map(|m| m.abs_diff(*p)).min().unwrap();
        assert!(
            nearest <= TEN_MS,
            "pico da saída em {p}: {nearest} amostras"
        );
    }
}

#[test]
fn saida_invalidada_reabre_e_segue_gravando() {
    let source = Arc::new(FakeSource::with_speed(Script::tones(), SPEED));
    let t = now_ms(&source);
    source.edit(|s| s.system.fail = Some((t + 2_000, EventKind::Invalidated)));
    let (rec, _dir, _done) = recorder(source.clone(), Limits::default());
    rec.start(&Choice::default()).unwrap();
    assert!(wait(60, || elapsed(&rec) >= 5_000));
    let r = rec.stop().unwrap();
    assert_eq!(r.end, End::User);
    assert_eq!(source.opens(Track::System), 2);
    assert_eq!(source.opens(Track::Mic), 1);
    assert_eq!(r.stats[1].reopens, 1);
    let sys = wav(&r.dir, Track::System);
    // Depois de reaberta (aos 2 s), a saída continua com o tom.
    let after = &sys[3 * RATE as usize..4 * RATE as usize];
    assert!(rms(after) > 0.05, "{}", rms(after));
}

#[test]
fn flush_a_cada_5s_deixa_o_wav_legivel_durante_a_gravacao() {
    let source = Arc::new(FakeSource::with_speed(Script::tones(), SPEED));
    let (rec, dir, _done) = recorder(source, Limits::default());
    let Status::Recording { id, .. } = rec.start(&Choice::default()).unwrap() else {
        panic!("não começou");
    };
    let path = dir.path().join("gravacoes").join(id.to_string());
    // Dois flushes (5 s e 10 s): mesmo com a escrita atrasada pela máquina ocupada (o relógio
    // falso corre 5×), o segundo já tem mais de 5 s.
    assert!(wait(60, || elapsed(&rec) >= 11_000));
    let frames = storage::wav_frames(&path, Track::Mic).unwrap();
    assert!(frames >= 5 * u64::from(RATE), "{frames} frames");
    rec.stop().unwrap();
}

#[test]
fn teto_de_duracao_para_sozinho() {
    let source = Arc::new(FakeSource::with_speed(Script::tones(), SPEED));
    let limits = Limits {
        max: Duration::from_secs(2),
        ..Limits::default()
    };
    let (rec, _dir, done) = recorder(source, limits);
    rec.start(&Choice::default()).unwrap();
    assert!(wait(60, || idle(&rec)));
    let r = done.lock().unwrap()[0].clone();
    assert_eq!(r.end, End::Limit);
    // A escrita confere o teto a cada passada; numa máquina ocupada, a passada atrasa (e o
    // relógio acelerado multiplica o atraso), mas a gravação para sozinha logo depois.
    assert!(
        (1_900..4_500).contains(&r.duration_ms),
        "{} ms",
        r.duration_ms
    );
}

#[test]
fn disco_quase_cheio_para_e_pouco_disco_nao_comeca() {
    let free = Arc::new(AtomicU64::new(10 << 30));
    let limits = Limits {
        disk_every: Duration::from_secs(1),
        free_space: {
            let free = free.clone();
            Arc::new(move |_: &Path| Some(free.load(Ordering::Relaxed)))
        },
        ..Limits::default()
    };
    let source = Arc::new(FakeSource::with_speed(Script::tones(), SPEED));
    let (rec, dir, done) = recorder(source.clone(), limits);
    rec.start(&Choice::default()).unwrap();
    assert!(wait(60, || elapsed(&rec) >= 2_000));
    free.store(100 << 20, Ordering::Relaxed);
    assert!(wait(60, || idle(&rec)));
    assert_eq!(done.lock().unwrap()[0].end, End::Disk);

    // Abaixo de 1 GB livre, o Gravar é recusado antes de abrir qualquer stream.
    let opens = source.opens(Track::Mic);
    assert_eq!(rec.start(&Choice::default()), Err(Error::Disk));
    assert_eq!(source.opens(Track::Mic), opens);
    assert_eq!(dirs(&dir.path().join("gravacoes")).len(), 1);
}

fn dirs(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .map(|e| e.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

#[test]
fn uma_gravacao_por_vez() {
    let source = Arc::new(FakeSource::with_speed(Script::tones(), SPEED));
    let (rec, _dir, _done) = recorder(source, Limits::default());
    assert_eq!(rec.stop(), Err(Error::NotRecording));
    rec.start(&Choice::default()).unwrap();
    assert_eq!(rec.start(&Choice::default()), Err(Error::Busy));
    assert!(rec.stop().is_ok());
    assert_eq!(rec.stop(), Err(Error::NotRecording));
}

#[test]
fn microfone_bloqueado_nao_deixa_pasta() {
    let mut script = Script::tones();
    script.mic.open_error = Some(Error::MicDenied);
    let source = Arc::new(FakeSource::with_speed(script, SPEED));
    let (rec, dir, done) = recorder(source.clone(), Limits::default());
    assert_eq!(rec.start(&Choice::default()), Err(Error::MicDenied));
    assert!(dirs(&dir.path().join("gravacoes")).is_empty());
    assert!(idle(&rec) && done.lock().unwrap().is_empty());
    assert_eq!(source.opens(Track::System), 0);
}

#[test]
fn encerramento_fecha_os_wavs_e_a_gravacao_volta_na_proxima_abertura() {
    let source = Arc::new(FakeSource::with_speed(Script::tones(), SPEED));
    let (rec, dir, done) = recorder(source, Limits::default());
    rec.start(&Choice::default()).unwrap();
    assert!(wait(60, || elapsed(&rec) >= 3_000));
    let r = rec.finish_on_exit().unwrap();
    assert_eq!(r.end, End::Exit);
    assert!(done.lock().unwrap().is_empty(), "sem job no encerramento");
    let found = recover(&dir.path().join("gravacoes"));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, r.id);
    assert!(found[0].duration_ms >= 3_000);
}

#[test]
fn dispositivos_simulados() {
    let source = Arc::new(FakeSource::new(Script::tones()));
    let (rec, _dir, _done) = recorder(source, Limits::default());
    let devices = rec.devices().unwrap();
    assert!(devices.inputs.iter().any(|d| d.default));
    assert!(devices.inputs.iter().any(|d| d.bluetooth));
    assert_eq!(devices.outputs.len(), 1);
}

/// Hardware: o microfone e o loopback reais desta máquina.
///
/// Nesta máquina o microfone (matriz Intel Smart Sound) cancela no próprio DSP o que os
/// alto-falantes tocam, então o eco não serve de marcador entre as trilhas. O alinhamento
/// é conferido por marcadores que não dependem dele: na trilha do sistema, o início de cada
/// trecho tocado contra o instante em que o teste mandou tocar; nas duas, o erro contra o
/// relógio dos pacotes e a deriva do dispositivo. A defasagem do eco é só informativa.
mod hardware {
    use std::sync::atomic::AtomicBool;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use super::*;
    use scriba::capture::CpalSource;

    /// Onde ficam as trilhas medidas, para a etapa 6.2 transcrever.
    fn out_dir() -> PathBuf {
        let dir = std::env::temp_dir().join("scriba-gravacao");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// O trecho da audiência da Câmara (fora do repositório), em 16 kHz mono.
    fn camara() -> Vec<f32> {
        let path = std::env::var("SCRIBA_REC_AUDIO")
            .unwrap_or_else(|_| r"C:\dev\audio-camara\trecho.wav".into());
        scriba::media::extract_audio(Path::new(&path), |_| {}, &AtomicBool::new(false))
            .expect("SCRIBA_REC_AUDIO: um áudio para tocar nos alto-falantes")
    }

    /// Toca amostras de 16 kHz mono na saída padrão (cada uma repetida na taxa do
    /// dispositivo). Devolve o stream, que para quando sai de cena, e o instante do pedido no
    /// relógio da fonte.
    fn play(source: &CpalSource, samples: Vec<f32>) -> (cpal::Stream, u64) {
        let device = cpal::default_host()
            .default_output_device()
            .expect("sem saída");
        let config = device.default_output_config().unwrap();
        let (rate, channels) = (config.sample_rate(), usize::from(config.channels()));
        let mut pos = 0f64;
        let step = 16_000.0 / f64::from(rate);
        let stream = device
            .build_output_stream(
                config.config(),
                move |data: &mut [f32], _| {
                    for frame in data.chunks_mut(channels) {
                        let s = samples.get(pos as usize).copied().unwrap_or(0.0);
                        frame.fill(s);
                        pos += step;
                    }
                },
                |e| eprintln!("erro tocando: {e}"),
                None,
            )
            .unwrap();
        let at = source.now_ns();
        stream.play().unwrap();
        (stream, at)
    }

    /// Toca por `secs` segundos e para; devolve o instante do pedido.
    fn play_for(source: &CpalSource, samples: Vec<f32>, secs: u64) -> u64 {
        let (stream, at) = play(source, samples);
        std::thread::sleep(Duration::from_secs(secs));
        drop(stream);
        at
    }

    /// O primeiro instante (s) a partir de `from` em que a trilha sai do silêncio digital.
    fn onset(samples: &[f32], from: f64) -> f64 {
        let start = (from * 32_000.0) as usize;
        let i = samples[start..]
            .iter()
            .position(|s| s.abs() > 0.01)
            .expect("o trecho tocado não chegou ao loopback");
        (start + i) as f64 / 32_000.0
    }

    /// A defasagem (ms) que melhor casa `mic` com `sys` entre `from` e `to` segundos, de 0 a
    /// 300 ms, pela correlação cruzada a 8 kHz, e a força da correlação (0..1).
    fn echo_lag(mic: &[f32], sys: &[f32], from: f64, to: f64) -> (f64, f64) {
        let down =
            |s: &[f32]| -> Vec<f32> { s.chunks(4).map(|c| c.iter().sum::<f32>() / 4.0).collect() };
        let (mic, sys) = (down(mic), down(sys));
        let (a, b) = ((from * 8_000.0) as usize, (to * 8_000.0) as usize);
        let x = &sys[a..b];
        let energy = |s: &[f32]| s.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
        let mut best = (0.0, 0.0);
        for lag in 0..2_400 {
            let y = &mic[a + lag..b + lag];
            let dot: f64 = x
                .iter()
                .zip(y)
                .map(|(p, q)| f64::from(*p) * f64::from(*q))
                .sum();
            let corr = dot / (energy(x) * energy(y)).max(1e-12);
            if corr > best.1 {
                best = (lag as f64 / 8.0, corr);
            }
        }
        best
    }

    /// Imprime as estatísticas e confere o que vale em qualquer gravação real: nenhum pacote
    /// perdido no ring e cada trilha a ±10 ms do relógio dos pacotes.
    fn report(r: &Recording) {
        for (track, s) in Track::ALL.iter().zip(r.stats) {
            let ppm =
                (s.received as f64 * 1e9 / (s.span_ns as f64 * f64::from(s.rate)) - 1.0) * 1e6;
            println!(
                "{track:?}: {} Hz, abertura {:.0} ms, deriva do dispositivo {ppm:+.0} ppm, silêncio inserido {:.1} ms em {} vezes, descartado {:.1} ms, erro máximo {:.2} ms, pacotes perdidos {}, reaberturas {}",
                s.rate,
                s.startup as f64 / 32.0,
                s.filled as f64 / 32.0,
                s.fills,
                s.dropped as f64 / 32.0,
                s.max_error as f64 / 32.0,
                s.lost,
                s.reopens
            );
            assert_eq!(s.lost, 0, "{track:?}: pacotes perdidos no ring");
            assert!(
                s.max_error <= TEN_MS as u64,
                "{track:?} fora do relógio: {s:?}"
            );
        }
    }

    /// O mesmo trecho de 15 s da Câmara, 15 s sem nada tocando, o trecho de novo; no fim, um
    /// stream só toca [3 s][4 s de zeros][os mesmos 3 s]. As trilhas terminam juntas, o
    /// loopback não parou no silêncio, cada trilha fica a ±10 ms do relógio, e os dois trechos
    /// do fim estão a 7 s exatos (±10 ms) na trilha do sistema.
    #[test]
    #[ignore = "hardware"]
    fn mic_e_loopback_reais_com_silencio_no_meio() {
        let clip = camara()[..15 * 16_000].to_vec();
        let mut marks = clip[..3 * 16_000].to_vec();
        marks.extend(std::iter::repeat_n(0.0, 4 * 16_000));
        marks.extend_from_slice(&clip[..3 * 16_000]);
        let root = out_dir().join("silencio");
        let _ = std::fs::remove_dir_all(&root);
        let source = Arc::new(CpalSource::new());
        let rec = Recorder::new(source.clone(), root, Limits::default());
        let Status::Recording { mic, output, .. } = rec.start(&Choice::default()).unwrap() else {
            panic!("não começou");
        };
        println!("microfone: {mic}; saída: {output}");
        std::thread::sleep(Duration::from_secs(1));
        let at_a = play_for(&source, clip.clone(), 15);
        // Nada toca além do silêncio do próprio Recorder.
        std::thread::sleep(Duration::from_secs(15));
        let at_b = play_for(&source, clip, 15);
        std::thread::sleep(Duration::from_secs(1));
        play_for(&source, marks, 11);
        let r = rec.stop().unwrap();
        report(&r);
        let (mic, sys) = (wav(&r.dir, Track::Mic), wav(&r.dir, Track::System));
        println!(
            "durações: mic {:.3} s, sistema {:.3} s; arquivos em {}",
            mic.len() as f64 / 32_000.0,
            sys.len() as f64 / 32_000.0,
            r.dir.display()
        );
        assert!((mic.len() as i64 - sys.len() as i64).abs() <= TEN_MS as i64);
        assert!(
            r.stats[1].filled < u64::from(RATE) / 100,
            "o loopback parou no silêncio: {:?}",
            r.stats[1]
        );
        let onset_a = onset(&sys, 0.0);
        let onset_b = onset(&sys, onset_a + 20.0);
        let asked = (at_b - at_a) as f64 / 1e9;
        println!(
            "trecho 2 depois do 1: {:.4} s no WAV, {asked:.4} s entre os pedidos ({:+.1} ms, inclui a abertura de cada stream)",
            onset_b - onset_a,
            (onset_b - onset_a - asked) * 1e3
        );
        let mark_1 = onset(&sys, onset_b + 16.0);
        let mark_2 = onset(&sys, mark_1 + 3.5);
        let distance = mark_2 - mark_1;
        println!(
            "marcadores do fim: {distance:.4} s no WAV, 7 s tocados ({:+.2} ms)",
            (distance - 7.0) * 1e3
        );
        assert!(
            (distance - 7.0).abs() <= 0.010,
            "a trilha do sistema desalinhou"
        );
        let before = echo_lag(&mic, &sys, onset_a + 2.0, onset_a + 12.0);
        let after = echo_lag(&mic, &sys, onset_b + 2.0, onset_b + 12.0);
        println!(
            "eco (informativo): {:.1} ms (correlação {:.2}) antes do silêncio, {:.1} ms ({:.2}) depois",
            before.0, before.1, after.0, after.1
        );
    }

    /// `SCRIBA_REC_MIN` minutos (padrão 60), com 20 s da Câmara no começo e no fim: pico de
    /// RAM, deriva de cada dispositivo e a distância entre os dois trechos na trilha do
    /// sistema contra a dos pedidos de tocar.
    #[test]
    #[ignore = "hardware"]
    fn gravacao_longa() {
        let minutes: u64 = std::env::var("SCRIBA_REC_MIN")
            .ok()
            .and_then(|m| m.parse().ok())
            .unwrap_or(60);
        // A tela acesa durante o teste: com ela apagada, o Modern Standby deste notebook
        // suspende tudo (na 1ª tentativa, a "1 h" durou 10 h de suspensões).
        // SAFETY: só muda o pedido desta thread ao gerenciador de energia.
        unsafe {
            use windows_sys::Win32::System::Power::{
                ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
            };
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED);
        }
        let root = out_dir().join("longa");
        let _ = std::fs::remove_dir_all(&root);
        let before = common::working_set();
        let sampler = common::Sampler::start();
        let source = Arc::new(CpalSource::new());
        let rec = Recorder::new(source.clone(), root, Limits::default());
        rec.start(&Choice::default()).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        let gap = marked(&source, minutes * 60 - 2);
        std::thread::sleep(Duration::from_secs(1));
        let r = rec.stop().unwrap();
        let peak = sampler.stop();
        report(&r);
        println!(
            "RAM: {} MB antes, pico {} MB; {:.1} min gravados",
            before >> 20,
            peak >> 20,
            r.duration_ms as f64 / 60_000.0
        );
        check_marks(&r, gap);
    }

    /// Um stream só, por `secs` segundos: 3 s da Câmara, zeros, e os mesmos 3 s no fim. A
    /// distância entre os dois trechos é exata em amostras do dispositivo; devolve-a em s.
    fn marked(source: &CpalSource, secs: u64) -> f64 {
        let clip = camara()[..3 * 16_000].to_vec();
        let gap = secs - 3;
        let mut samples = clip.clone();
        samples.extend(std::iter::repeat_n(0.0, (gap as usize - 3) * 16_000));
        samples.extend_from_slice(&clip);
        play_for(source, samples, secs);
        gap as f64
    }

    /// Os dois trechos de [`marked`] na trilha do sistema estão à distância tocada, corrigida
    /// pela deriva do relógio da saída (o WAV segue o relógio dos pacotes), a ±10 ms.
    fn check_marks(r: &Recording, gap: f64) {
        let sys = wav(&r.dir, Track::System);
        let s = r.stats[Track::System.index()];
        let ratio = s.received as f64 * 1e9 / (s.span_ns as f64 * f64::from(s.rate));
        let expected = gap / ratio;
        let first = onset(&sys, 0.0);
        let second = onset(&sys, first + 3.5);
        let found = second - first;
        println!(
            "marcadores: {found:.4} s no WAV, {expected:.4} s esperados ({gap} s tocados, {:+.1} ms)",
            (found - expected) * 1e3
        );
        assert!(
            (found - expected).abs() <= 0.010,
            "a trilha do sistema desalinhou"
        );
    }

    /// Grava `SCRIBA_REC_CONCURRENT_MIN` minutos (padrão 3) enquanto o `small` transcreve
    /// `SCRIBA_LONG_AUDIO` na CPU: nenhum pacote perdido e as trilhas a ±10 ms do relógio.
    #[test]
    #[ignore = "hardware"]
    fn gravar_enquanto_transcreve_na_cpu() {
        use scriba::asr::{Asr, Models, Plan};
        let minutes: u64 = std::env::var("SCRIBA_REC_CONCURRENT_MIN")
            .ok()
            .and_then(|m| m.parse().ok())
            .unwrap_or(3);
        let long = std::env::var("SCRIBA_LONG_AUDIO").expect("SCRIBA_LONG_AUDIO: 1 h de áudio");
        let stop = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(false));
        let transcriber = std::thread::spawn({
            let (stop, busy) = (stop.clone(), busy.clone());
            move || {
                let audio = scriba::media::extract_audio(Path::new(&long), |_| {}, &stop).unwrap();
                let asr = Asr::new(vec![Plan::cpu()], Models::locate());
                let _ = asr.transcribe(&audio, |_| busy.store(true, Ordering::Relaxed), &stop);
            }
        });
        assert!(
            wait(120, || busy.load(Ordering::Relaxed)),
            "a transcrição não começou"
        );
        let root = out_dir().join("concorrente");
        let _ = std::fs::remove_dir_all(&root);
        let source = Arc::new(CpalSource::new());
        let rec = Recorder::new(source.clone(), root, Limits::default());
        rec.start(&Choice::default()).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        let gap = marked(&source, minutes * 60);
        let r = rec.stop().unwrap();
        let still_busy = !transcriber.is_finished();
        stop.store(true, Ordering::Relaxed);
        let _ = transcriber.join();
        assert!(still_busy, "a transcrição terminou antes da gravação");
        report(&r);
        check_marks(&r, gap);
    }
}
