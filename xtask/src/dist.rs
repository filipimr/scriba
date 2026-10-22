//! `cargo xtask dist`: o build da release, o mesmo aqui e no `release.yml`. Baixa os modelos
//! que faltam, limpa o whisper.cpp quando as flags do `.cargo/config.toml` mudam, compila com
//! Vulkan e os modelos embutidos, copia para `dist/` com o SHA-256 e as licenças, confere as
//! DLLs e a VERSIONINFO e roda o `--self-test`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, bail, ensure};
use sha2::{Digest, Sha256};

/// Os modelos do exe completo: o large (GPU), o `small` (CPU) e o VAD.
const MODELS: &str = "large,small,silero";
const FEATURES: &str = "vulkan embedded-model";
/// O GitHub recusa assets acima de 2 GB; acima disto, algo entrou que não devia.
const MAX_MB: u64 = 1024;

/// DLLs do Windows que o exe pode importar direto (sempre presentes num Windows 10/11 x64);
/// `api-ms-win-*` e `ext-ms-*` também. Qualquer outra (FFmpeg, `vcruntime140.dll`) quebra o
/// exe único.
const SYSTEM_DLLS: &[&str] = &[
    "advapi32.dll",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "combase.dll",
    "crypt32.dll",
    "gdi32.dll",
    "kernel32.dll",
    "mfplat.dll",
    "mmdevapi.dll",
    "ncrypt.dll",
    "ntdll.dll",
    "ole32.dll",
    "oleaut32.dll",
    "propsys.dll",
    "secur32.dll",
    "shell32.dll",
    "shlwapi.dll",
    "user32.dll",
    "uxtheme.dll",
    "winmm.dll",
    "ws2_32.dll",
];

pub struct Options {
    /// Pula o `--self-test` (o CI o roda em passos próprios, com as variáveis do smoke).
    pub skip_self_test: bool,
    /// Não limpa o whisper.cpp: o cache do CI já é separado pelo hash do
    /// `.cargo/config.toml` (e o carimbo não sobrevive à poda do rust-cache).
    pub trust_cache: bool,
}

pub fn run(root: &Path, opts: &Options) -> anyhow::Result<()> {
    let total = Instant::now();
    let mut step = Steps::default();

    step.run("modelos", || {
        crate::models::download(&root.join("models"), Some(MODELS))
    })?;

    let target = target_dir(root);
    if !opts.trust_cache {
        step.run("flags do whisper.cpp", || {
            clean_if_flags_changed(root, &target)
        })?;
    }

    step.run("build", || {
        cargo(root, &["build", "--release", "--features", FEATURES])
    })?;

    let dist = root.join("dist");
    let exe = dist.join("scriba.exe");
    step.run("dist/", || {
        fs::create_dir_all(&dist)?;
        fs::copy(target.join("release").join("scriba.exe"), &exe)?;
        let hash = hex::encode(Sha256::digest(fs::read(&exe)?));
        fs::write(
            dist.join("scriba.exe.sha256"),
            format!("{hash}  scriba.exe\n"),
        )?;
        println!("SHA-256 {hash}");
        Ok(())
    })?;

    step.run("licenças", || {
        let out = dist.join("THIRD_PARTY_LICENSES.html");
        let status = Command::new("cargo")
            .current_dir(root)
            .args(["about", "generate", "about.hbs", "-o"])
            .arg(&out)
            .status()
            .context(
                "cargo about (instale com `cargo install cargo-about --locked --features cli`)",
            )?;
        ensure!(status.success(), "cargo about falhou");
        Ok(())
    })?;

    step.run("DLLs (dumpbin)", || check_imports(&exe))?;
    step.run("VERSIONINFO", || check_version_info(root, &exe))?;
    if !opts.skip_self_test {
        step.run("--self-test", || self_test(&exe))?;
    }

    let mb = fs::metadata(&exe)?.len() / (1 << 20);
    ensure!(mb <= MAX_MB, "o exe tem {mb} MB, acima de {MAX_MB} MB");
    println!();
    for (name, secs) in &step.times {
        println!("{name:<22} {secs:>7.1} s");
    }
    println!(
        "{:<22} {:>7.1} s\n{} ({mb} MB)",
        "total",
        total.elapsed().as_secs_f64(),
        exe.display()
    );
    Ok(())
}

#[derive(Default)]
struct Steps {
    times: Vec<(&'static str, f64)>,
}

impl Steps {
    fn run(
        &mut self,
        name: &'static str,
        f: impl FnOnce() -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        println!("== {name}");
        let t = Instant::now();
        f().with_context(|| format!("dist: {name}"))?;
        self.times.push((name, t.elapsed().as_secs_f64()));
        Ok(())
    }
}

fn cargo(root: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("cargo")
        .current_dir(root)
        .args(args)
        .status()?;
    ensure!(status.success(), "cargo {} falhou", args.join(" "));
    Ok(())
}

pub fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
}

/// O build script do whisper-rs-sys não rastreia as flags `GGML_*`/`CMAKE_*`: um whisper.cpp
/// de flags antigas (sem `/O2`) passaria. O carimbo guarda o hash do `.cargo/config.toml`.
fn clean_if_flags_changed(root: &Path, target: &Path) -> anyhow::Result<()> {
    let config = fs::read(root.join(".cargo").join("config.toml"))?;
    let hash = hex::encode(Sha256::digest(&config));
    let stamp = target.join("release").join(".scriba-config-hash");
    if fs::read_to_string(&stamp).ok().as_deref() == Some(hash.as_str()) {
        println!("flags iguais às do último build");
        return Ok(());
    }
    println!("flags mudaram (ou primeiro build): limpando o whisper-rs-sys");
    cargo(root, &["clean", "-p", "whisper-rs-sys", "--release"])?;
    fs::create_dir_all(stamp.parent().unwrap())?;
    fs::write(&stamp, hash)?;
    Ok(())
}

/// O `dumpbin.exe` do Visual Studio mais novo (pelo `vswhere`).
fn dumpbin() -> anyhow::Result<PathBuf> {
    let programs = std::env::var_os("ProgramFiles(x86)").context("ProgramFiles(x86)")?;
    let vswhere = Path::new(&programs).join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    let out = Command::new(&vswhere)
        .args(["-latest", "-products", "*", "-property", "installationPath"])
        .output()
        .with_context(|| format!("rodando {}", vswhere.display()))?;
    let vs = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let tools = vs.join(r"VC\Tools\MSVC");
    let mut versions: Vec<PathBuf> = fs::read_dir(&tools)
        .with_context(|| format!("lendo {}", tools.display()))?
        .flatten()
        .map(|e| e.path())
        .collect();
    versions.sort();
    versions
        .iter()
        .rev()
        .map(|v| v.join(r"bin\Hostx64\x64\dumpbin.exe"))
        .find(|p| p.is_file())
        .context("dumpbin.exe não encontrado")
}

/// As DLLs importadas direto e as de delay-load, pela saída do `dumpbin /dependents`.
pub fn parse_dependents(output: &str) -> (Vec<String>, Vec<String>) {
    let (mut direct, mut delay) = (Vec::new(), Vec::new());
    let mut current: Option<&mut Vec<String>> = None;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Image has the following dependencies") {
            current = Some(&mut direct);
        } else if trimmed.starts_with("Image has the following delay load dependencies") {
            current = Some(&mut delay);
        } else if trimmed.starts_with("Summary") {
            current = None;
        } else if let Some(list) = current.as_deref_mut()
            && trimmed.to_ascii_lowercase().ends_with(".dll")
        {
            list.push(trimmed.to_ascii_lowercase());
        }
    }
    (direct, delay)
}

/// Só DLLs do Windows direto, e o `vulkan-1.dll` só com delay-load.
pub fn check_dependents(direct: &[String], delay: &[String]) -> anyhow::Result<()> {
    let foreign: Vec<&String> = direct
        .iter()
        .filter(|d| {
            !(SYSTEM_DLLS.contains(&d.as_str())
                || d.starts_with("api-ms-win-")
                || d.starts_with("ext-ms-"))
        })
        .collect();
    ensure!(
        foreign.is_empty(),
        "o exe importa DLLs que não são do Windows: {foreign:?}"
    );
    ensure!(
        delay == ["vulkan-1.dll"],
        "o delay-load deveria ter só o vulkan-1.dll, tem {delay:?}"
    );
    Ok(())
}

fn check_imports(exe: &Path) -> anyhow::Result<()> {
    let out = Command::new(dumpbin()?)
        .arg("/dependents")
        .arg(exe)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (direct, delay) = parse_dependents(&text);
    println!("diretas: {}", direct.join(", "));
    println!("delay-load: {}", delay.join(", "));
    check_dependents(&direct, &delay)
}

/// A VERSIONINFO pelo PowerShell (`FileVersionInfo`): nome, descrição e a versão do
/// `Cargo.toml`.
fn check_version_info(root: &Path, exe: &Path) -> anyhow::Result<()> {
    let script = format!(
        "$v = (Get-Item -LiteralPath '{}').VersionInfo; \
         \"$($v.ProductName)|$($v.FileDescription)|$($v.ProductVersion)\"",
        exe.display()
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()?;
    // A saída do PowerShell 5.1 sai na página de código do console; o texto comparado é ASCII
    // fora o ":" da descrição.
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let version = package_version(root)?;
    let mut parts = text.split('|');
    let (product, description, product_version) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    println!("{product} · {description} · {product_version}");
    ensure!(product == "Scriba", "ProductName {product:?}");
    ensure!(
        description.starts_with("Scriba") && description.ends_with("grava e transcreve"),
        "FileDescription {description:?}"
    );
    ensure!(
        product_version.starts_with(&version),
        "ProductVersion {product_version:?}, esperada {version}"
    );
    Ok(())
}

/// A versão do pacote `scriba` no `Cargo.toml` da raiz.
fn package_version(root: &Path) -> anyhow::Result<String> {
    let toml = fs::read_to_string(root.join("Cargo.toml"))?;
    toml.lines()
        .skip_while(|l| l.trim() != "[package]")
        .find_map(|l| {
            l.trim()
                .strip_prefix("version = \"")
                .and_then(|v| v.strip_suffix('"'))
                .map(str::to_string)
        })
        .context("versão do [package] no Cargo.toml")
}

/// O exe é de janela: a saída só aparece num pipe, e o código de saída vale.
fn self_test(exe: &Path) -> anyhow::Result<()> {
    let out = Command::new(exe).arg("--self-test").output()?;
    print!("{}", String::from_utf8_lossy(&out.stdout));
    eprint!("{}", String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        bail!("--self-test saiu com {}", out.status);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMPBIN: &str = "\
Dump of file scriba.exe

File Type: EXECUTABLE IMAGE

  Image has the following dependencies:

    KERNEL32.dll
    api-ms-win-core-synch-l1-2-0.dll
    Secur32.dll

  Image has the following delay load dependencies:

    vulkan-1.dll

  Summary

        1000 .data
";

    #[test]
    fn le_o_dumpbin() {
        let (direct, delay) = parse_dependents(DUMPBIN);
        assert_eq!(
            direct,
            [
                "kernel32.dll",
                "api-ms-win-core-synch-l1-2-0.dll",
                "secur32.dll"
            ]
        );
        assert_eq!(delay, ["vulkan-1.dll"]);
        assert!(check_dependents(&direct, &delay).is_ok());
    }

    #[test]
    fn recusa_dll_de_fora_e_vulkan_direto() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let vulkan = s(&["vulkan-1.dll"]);
        let err = check_dependents(&s(&["kernel32.dll", "avcodec-61.dll"]), &vulkan).unwrap_err();
        assert!(err.to_string().contains("avcodec-61.dll"), "{err}");
        assert!(check_dependents(&s(&["vcruntime140.dll"]), &vulkan).is_err());
        assert!(check_dependents(&s(&["kernel32.dll", "vulkan-1.dll"]), &[]).is_err());
        assert!(check_dependents(&s(&["kernel32.dll"]), &[]).is_err());
    }

    #[test]
    fn versao_do_pacote() {
        let v = package_version(&crate::root()).unwrap();
        assert!(v.split('.').count() == 3, "{v}");
    }
}
