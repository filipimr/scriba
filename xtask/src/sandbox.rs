//! `cargo xtask sandbox-test`: roda o `scripts/smoke.ps1 -Sandbox` no Windows Sandbox, uma
//! máquina limpa, sem rede e (sem `--gpu`) sem GPU, e lê o resultado.
//!
//! Monta `<target>/sandbox/<execução>/{entrada,saida}`: a entrada (só leitura no Sandbox) leva
//! o exe, o `smoke.ps1`, o `fala.mp4`, o `fala.wav` e o `rodar.ps1` do `LogonCommand`; a saída
//! recebe o `resultado.json` e os logs. O `rodar.ps1` desliga o Sandbox no fim.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail, ensure};

const SANDBOX_EXE: &str = r"C:\Windows\System32\WindowsSandbox.exe";
const INSIDE: &str = r"C:\Scriba";
const TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub struct Options {
    pub exe: Option<PathBuf>,
    /// Liga a vGPU do Sandbox (só para registrar: o critério é sem GPU).
    pub gpu: bool,
}

pub fn run(root: &Path, opts: &Options) -> anyhow::Result<()> {
    ensure!(
        Path::new(SANDBOX_EXE).is_file(),
        "o Windows Sandbox não está habilitado: num PowerShell de administrador, rode \
         `powershell -ExecutionPolicy Bypass -File scripts\\setup.ps1 -Sandbox` e reinicie o Windows"
    );
    let exe = opts
        .exe
        .clone()
        .unwrap_or_else(|| root.join("dist").join("scriba.exe"));
    ensure!(
        exe.is_file(),
        "{} não existe (rode `cargo xtask dist` ou passe --exe)",
        exe.display()
    );

    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let run_dir = crate::dist::target_dir(root)
        .join("sandbox")
        .join(stamp.to_string());
    let (input, output) = (run_dir.join("entrada"), run_dir.join("saida"));
    fs::create_dir_all(&input)?;
    fs::create_dir_all(&output)?;
    fs::copy(&exe, input.join("scriba.exe")).context("copiando o exe")?;
    fs::copy(root.join("scripts/smoke.ps1"), input.join("smoke.ps1"))?;
    for fixture in ["fala.mp4", "fala.wav"] {
        fs::copy(
            root.join("tests/fixtures").join(fixture),
            input.join(fixture),
        )?;
    }
    fs::write(input.join("rodar.ps1"), logon_script(opts.gpu))?;
    let wsb = run_dir.join("scriba.wsb");
    fs::write(&wsb, wsb_config(&input, &output, opts.gpu))?;
    println!("Sandbox: {}", run_dir.display());

    let started = Instant::now();
    Command::new(SANDBOX_EXE)
        .arg(&wsb)
        .spawn()
        .context("abrindo o Windows Sandbox")?;
    let result = output.join("resultado.json");
    let done = output.join("fim.txt");
    while !done.is_file() {
        if started.elapsed() > TIMEOUT {
            bail!(
                "o Sandbox não terminou em {} min (veja {})",
                TIMEOUT.as_secs() / 60,
                output.display()
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    println!("pronto em {:.0} s", started.elapsed().as_secs_f64());
    let text = fs::read_to_string(&result).with_context(|| format!("sem {}", result.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}')).context("resultado.json")?;
    report(&json);
    println!("logs e resultado em {}", output.display());
    ensure!(json["ok"] == true, "o smoke falhou no Sandbox");
    Ok(())
}

fn report(json: &serde_json::Value) {
    for step in json["passos"].as_array().into_iter().flatten() {
        let mark = match step["ok"].as_bool() {
            Some(true) => "ok  ",
            Some(false) => "FALHA",
            None => "pula",
        };
        println!(
            "{mark} {} ({} s) {}",
            step["nome"].as_str().unwrap_or_default(),
            step["segundos"],
            step["detalhe"].as_str().unwrap_or_default()
        );
    }
    if let Some(info) = json["info"].as_object() {
        for key in [
            "windows",
            "ram_gb",
            "vulkan_dll",
            "gpu",
            "abertura_s",
            "gravacao",
        ] {
            if let Some(v) = info.get(key) {
                println!("{key}: {v}");
            }
        }
    }
}

/// O `LogonCommand`: o smoke com `-Sandbox`, a marca do fim e o desligamento. Só ASCII: o
/// Windows PowerShell 5.1 lê um script sem BOM na página de código do sistema.
fn logon_script(gpu: bool) -> String {
    let expect = if gpu {
        ""
    } else {
        "$env:SCRIBA_SELF_TEST_EXPECT = 'cpu'\r\n"
    };
    format!(
        "{expect}$in = '{INSIDE}\\entrada'\r\n\
         $out = '{INSIDE}\\saida'\r\n\
         & \"$in\\smoke.ps1\" -Exe \"$in\\scriba.exe\" -Fixtures $in -Out $out -Sandbox *> \"$out\\smoke.txt\"\r\n\
         \"$LASTEXITCODE\" | Out-File -Encoding ascii \"$out\\fim.txt\"\r\n\
         shutdown.exe /s /t 5\r\n"
    )
}

/// O `.wsb`: sem rede (prova que funciona offline), sem vGPU (máquina sem GPU), com o
/// microfone, a entrada só leitura e a saída gravável.
pub fn wsb_config(input: &Path, output: &Path, gpu: bool) -> String {
    let vgpu = if gpu { "Enable" } else { "Disable" };
    format!(
        "<Configuration>\r\n\
         \x20 <VGpu>{vgpu}</VGpu>\r\n\
         \x20 <Networking>Disable</Networking>\r\n\
         \x20 <AudioInput>Enable</AudioInput>\r\n\
         \x20 <MemoryInMB>8192</MemoryInMB>\r\n\
         \x20 <MappedFolders>\r\n\
         \x20   <MappedFolder>\r\n\
         \x20     <HostFolder>{}</HostFolder>\r\n\
         \x20     <SandboxFolder>{INSIDE}\\entrada</SandboxFolder>\r\n\
         \x20     <ReadOnly>true</ReadOnly>\r\n\
         \x20   </MappedFolder>\r\n\
         \x20   <MappedFolder>\r\n\
         \x20     <HostFolder>{}</HostFolder>\r\n\
         \x20     <SandboxFolder>{INSIDE}\\saida</SandboxFolder>\r\n\
         \x20     <ReadOnly>false</ReadOnly>\r\n\
         \x20   </MappedFolder>\r\n\
         \x20 </MappedFolders>\r\n\
         \x20 <LogonCommand>\r\n\
         \x20   <Command>powershell.exe -NoProfile -ExecutionPolicy Bypass -File {INSIDE}\\entrada\\rodar.ps1</Command>\r\n\
         \x20 </LogonCommand>\r\n\
         </Configuration>\r\n",
        xml(&input.display().to_string()),
        xml(&output.display().to_string())
    )
}

/// Um caminho como texto do XML (um `&` no nome da pasta quebraria o `.wsb`).
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsb_sem_rede_sem_gpu_e_entrada_so_leitura() {
        let wsb = wsb_config(Path::new(r"C:\t\e"), Path::new(r"C:\t\s"), false);
        for part in [
            "<VGpu>Disable</VGpu>",
            "<Networking>Disable</Networking>",
            "<AudioInput>Enable</AudioInput>",
            r"<HostFolder>C:\t\e</HostFolder>",
            r"<SandboxFolder>C:\Scriba\entrada</SandboxFolder>",
            "<ReadOnly>true</ReadOnly>",
            r"-File C:\Scriba\entrada\rodar.ps1",
        ] {
            assert!(wsb.contains(part), "{part}\n{wsb}");
        }
        assert!(wsb_config(Path::new("e"), Path::new("s"), true).contains("<VGpu>Enable</VGpu>"));
        assert!(
            wsb_config(Path::new(r"C:\P&D\e"), Path::new("s"), false)
                .contains(r"<HostFolder>C:\P&amp;D\e</HostFolder>")
        );
    }

    #[test]
    fn script_do_logon_e_ascii_e_desliga() {
        let script = logon_script(false);
        assert!(script.is_ascii());
        assert!(script.contains("SCRIBA_SELF_TEST_EXPECT = 'cpu'"));
        assert!(script.contains("-Sandbox") && script.contains("fim.txt"));
        assert!(script.ends_with("shutdown.exe /s /t 5\r\n"));
        assert!(!logon_script(true).contains("EXPECT"));
    }
}
