//! Dependências de link do FFmpeg estático (vcpkg) que o ffmpeg-sys-next não declara
//! quando usa FFMPEG_DIR: x264 e bibliotecas do Windows (schannel, CNG, COM, Media Foundation).

fn main() {
    println!("cargo:rustc-link-lib=static=libx264");
    for lib in [
        "secur32", "crypt32", "ncrypt", "bcrypt", "ole32", "user32", "mfuuid", "strmiids", "mfplat",
    ] {
        println!("cargo:rustc-link-lib={lib}");
    }
}
