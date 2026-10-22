//! Dependências de link do FFmpeg estático (vcpkg) que o ffmpeg-sys-next não declara
//! quando usa FFMPEG_DIR: x264 e bibliotecas do Windows (schannel, CNG, COM, Media Foundation).
//! E os recursos do exe (`winresource`): o ícone do app, os dois da bandeja, a VERSIONINFO
//! e o manifesto.

/// pt-BR (`LANG_PORTUGUESE`, `SUBLANG_PORTUGUESE_BRAZILIAN`).
const PT_BR: u16 = 0x0416;

/// `asInvoker` (sem UAC), DPI por monitor (o "Salvar como" e as `MessageBox` nítidos em
/// 150%), os Common Controls 6 (o estilo atual das janelas do sistema), Windows 10 e 11, e
/// caminhos longos.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Scriba" version="1.0.0.0" processorArchitecture="amd64"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

fn main() {
    println!("cargo:rustc-link-lib=static=libx264");
    for lib in [
        "secur32", "crypt32", "ncrypt", "bcrypt", "ole32", "user32", "mfuuid", "strmiids", "mfplat",
    ] {
        println!("cargo:rustc-link-lib={lib}");
    }
    // O exe abre sem o `vulkan-1.dll` (PC sem driver de GPU): o Windows só o carrega na
    // primeira chamada, e os hooks de `src/delay_load.rs` cobrem a falta.
    if std::env::var_os("CARGO_FEATURE_VULKAN").is_some() {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:vulkan-1.dll");
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }
    resources();
}

fn resources() {
    // Os ids batem com `src/tray/win32.rs`; o 1 (o menor) é o ícone do exe.
    let icons = [
        ("assets/brand/scriba.ico", "1"),
        ("assets/brand/bandeja-escura.ico", "2"),
        ("assets/brand/bandeja-clara.ico", "3"),
    ];
    for (path, _) in icons {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    let mut res = winresource::WindowsResource::new();
    for (path, id) in icons {
        res.set_icon_with_id(path, id);
    }
    res.set_language(PT_BR)
        .set("ProductName", "Scriba")
        .set("FileDescription", "Scriba: grava e transcreve")
        .set("CompanyName", "Scriba")
        .set("LegalCopyright", "GPL-3.0-or-later")
        .set("OriginalFilename", "scriba.exe")
        .set("InternalName", "scriba")
        .set_manifest(MANIFEST);
    res.compile()
        .expect("compilando os recursos do exe (rc.exe)");
}
