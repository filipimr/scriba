//! Spike B: FFmpeg estático extrai áudio para 16 kHz mono f32 e codifica H.264.

use std::path::Path;
use std::process::Command;

use ffmpeg::format::{Pixel, Sample, sample};
use ffmpeg::software::resampling;
use ffmpeg::util::frame;
use ffmpeg::{ChannelLayout, Dictionary, Rational, codec, encoder, media};
use ffmpeg_next as ffmpeg;
use spikes::fixture;

/// Decodifica a melhor faixa de áudio e reamostra para 16 kHz mono f32.
fn extract_audio(path: &Path) -> Vec<f32> {
    ffmpeg::init().unwrap();
    let mut ictx = ffmpeg::format::input(path).unwrap();
    let stream = ictx.streams().best(media::Type::Audio).expect("sem áudio");
    let index = stream.index();
    let mut dec = codec::context::Context::from_parameters(stream.parameters())
        .unwrap()
        .decoder()
        .audio()
        .unwrap();
    // O resampler nasce do primeiro frame: o layout informado pelo decoder antes de
    // decodificar pode vir vazio (WAV mono) e o swr recusa com "Input changed".
    let mut rs: Option<resampling::Context> = None;
    let mut out = Vec::new();
    let mut push = |dec: &mut ffmpeg::decoder::Audio| {
        let mut decoded = frame::Audio::empty();
        while dec.receive_frame(&mut decoded).is_ok() {
            if decoded.channel_layout().is_empty() {
                decoded.set_channel_layout(ChannelLayout::default(i32::from(decoded.channels())));
            }
            let rs = rs.get_or_insert_with(|| {
                resampling::Context::get(
                    decoded.format(),
                    decoded.channel_layout(),
                    decoded.rate(),
                    Sample::F32(sample::Type::Packed),
                    ChannelLayout::MONO,
                    16_000,
                )
                .unwrap()
            });
            let mut res = frame::Audio::empty();
            rs.run(&decoded, &mut res).unwrap();
            out.extend_from_slice(&res.plane::<f32>(0)[..res.samples()]);
        }
    };
    for (s, packet) in ictx.packets() {
        if s.index() == index {
            dec.send_packet(&packet).unwrap();
            push(&mut dec);
        }
    }
    dec.send_eof().unwrap();
    push(&mut dec);
    out
}

#[test]
fn extrai_audio_16k_mono_de_todos_os_formatos() {
    let reference = extract_audio(&fixture("fala.wav")).len() as f64 / 16_000.0;
    for name in ["fala.mp4", "fala.mp3", "fala.m4a", "fala.ogg"] {
        let secs = extract_audio(&fixture(name)).len() as f64 / 16_000.0;
        println!("{name}: {secs:.2} s (wav {reference:.2} s)");
        // Codecs com perda acrescentam alguns ms de atraso/padding.
        assert!((secs - reference).abs() < 0.15, "{name}: {secs} s");
    }
}

#[test]
fn video_sem_audio_nao_tem_faixa() {
    ffmpeg::init().unwrap();
    let ictx = ffmpeg::format::input(&fixture("video-sem-audio.mp4")).unwrap();
    assert!(ictx.streams().best(media::Type::Audio).is_none());
}

/// Codifica 2 s de vídeo cinza 320x240 a 10 fps com `codec_name` em MP4.
fn encode_h264(codec_name: &str, out: &Path) {
    ffmpeg::init().unwrap();
    let codec = encoder::find_by_name(codec_name).unwrap_or_else(|| panic!("{codec_name} ausente"));
    let mut octx = ffmpeg::format::output(out).unwrap();
    let global_header = octx
        .format()
        .flags()
        .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
    let mut ost = octx.add_stream(codec).unwrap();
    let mut enc = codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .unwrap();
    let tb = Rational::new(1, 10);
    enc.set_width(320);
    enc.set_height(240);
    enc.set_format(Pixel::YUV420P);
    enc.set_time_base(tb);
    enc.set_frame_rate(Some(Rational::new(10, 1)));
    if global_header {
        enc.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    let mut enc = enc.open_as_with(codec, Dictionary::new()).unwrap();
    ost.set_parameters(&enc);
    ost.set_time_base(tb);
    octx.write_header().unwrap();
    let ost_tb = octx.stream(0).unwrap().time_base();

    let mut packets = 0;
    let mut drain = |enc: &mut encoder::Video, octx: &mut ffmpeg::format::context::Output| {
        let mut pkt = ffmpeg::Packet::empty();
        while enc.receive_packet(&mut pkt).is_ok() {
            pkt.set_stream(0);
            pkt.rescale_ts(tb, ost_tb);
            pkt.write_interleaved(octx).unwrap();
            packets += 1;
        }
    };
    let mut f = frame::Video::new(Pixel::YUV420P, 320, 240);
    for i in 0..20 {
        for p in 0..3 {
            f.data_mut(p).fill(128);
        }
        f.set_pts(Some(i));
        enc.send_frame(&f).unwrap();
        drain(&mut enc, &mut octx);
    }
    enc.send_eof().unwrap();
    drain(&mut enc, &mut octx);
    octx.write_trailer().unwrap();
    assert!(packets > 0, "{codec_name} não produziu pacotes");

    let ictx = ffmpeg::format::input(out).unwrap();
    let secs = ictx.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE);
    println!("{codec_name}: {packets} pacotes, {secs:.2} s");
    assert!((secs - 2.0).abs() <= 0.15, "duração {secs}");
}

#[test]
fn codifica_h264_com_x264() {
    encode_h264(
        "libx264",
        &std::env::temp_dir().join("scriba-spike-b-x264.mp4"),
    );
}

#[test]
#[ignore = "hardware"]
fn codifica_h264_com_nvenc() {
    encode_h264(
        "h264_nvenc",
        &std::env::temp_dir().join("scriba-spike-b-nvenc.mp4"),
    );
}

/// O executável de teste não pode depender de DLLs do FFmpeg.
#[test]
fn sem_dlls_do_ffmpeg() {
    let vswhere = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    let out = Command::new(vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-find",
            r"VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe",
        ])
        .output()
        .expect("vswhere");
    let dumpbin = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .expect("dumpbin não encontrado")
        .to_string();
    let exe = std::env::current_exe().unwrap();
    let deps = Command::new(dumpbin)
        .arg("/dependents")
        .arg(&exe)
        .output()
        .unwrap();
    let deps = String::from_utf8_lossy(&deps.stdout).to_lowercase();
    println!("{deps}");
    for dll in [
        "avcodec",
        "avformat",
        "avutil",
        "swresample",
        "swscale",
        "x264",
    ] {
        assert!(!deps.contains(dll), "depende de {dll}*.dll");
    }
}
