// A release é um app de janela: aberta pelo Explorer, não mostra console. O debug (`cargo run`,
// E2E, testes) continua de console, com Ctrl+C.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use scriba::app::{self, Options};
use scriba::config::Paths;

/// Os hooks do `delayimp` para o `vulkan-1.dll` (`/DELAYLOAD` no `build.rs`): ficam no exe,
/// e não na biblioteca, para o linker usá-los no lugar dos padrões do `delayimp.lib`.
#[cfg(feature = "vulkan")]
#[unsafe(no_mangle)]
#[used]
static __pfnDliFailureHook2: scriba::delay_load::Hook = Some(scriba::delay_load::failure_hook);

#[cfg(feature = "vulkan")]
#[unsafe(no_mangle)]
#[used]
static __pfnDliNotifyHook2: scriba::delay_load::Hook = Some(scriba::delay_load::notify_hook);

#[derive(Parser)]
#[command(version, about = "Grava e transcreve")]
struct Cli {
    /// Transcreve uma amostra embutida onde o app transcreveria (GPU ou CPU) e codifica um
    /// clipe curto com x264.
    #[arg(long)]
    self_test: bool,

    /// Não abre o navegador ao iniciar.
    #[arg(long)]
    no_open: bool,

    /// Não põe o ícone na bandeja (testes E2E).
    #[arg(long)]
    no_tray: bool,

    /// Porta local preferida; se estiver ocupada, usa outra livre. 0 = qualquer uma.
    #[arg(long, default_value_t = 47800)]
    port: u16,

    /// Dados internos do app (padrão: %LOCALAPPDATA%\Scriba).
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// Pasta das saídas (padrão: Documentos\Scriba).
    #[arg(long)]
    output_dir: Option<PathBuf>,

    /// Usa o pipeline falso, que não lê o arquivo (testes E2E).
    #[cfg(feature = "test-support")]
    #[arg(long)]
    fake_pipeline: bool,

    /// Grava tons simulados no lugar do microfone e do som do PC (testes E2E); o roteiro
    /// opcional fica em `<data-dir>/audio-falso.txt`.
    #[cfg(feature = "test-support")]
    #[arg(long)]
    fake_audio: bool,
}

fn main() -> ExitCode {
    // `--help`, `--version` e um erro de argumento: o texto vai para o console de quem chamou.
    let cli = Cli::try_parse().unwrap_or_else(|e| {
        scriba::console::attach_parent();
        e.exit()
    });
    if cli.self_test {
        scriba::console::attach_parent();
        return match scriba::self_test::run_default() {
            Ok(report) => {
                println!("self-test ok: {report}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("self-test falhou: {e:#}");
                ExitCode::FAILURE
            }
        };
    }
    let opts = Options {
        port: cli.port,
        open_browser: !cli.no_open,
        tray: !cli.no_tray,
        paths: Paths::resolve(cli.data_dir, cli.output_dir),
        #[cfg(feature = "test-support")]
        fake_pipeline: cli.fake_pipeline,
        #[cfg(feature = "test-support")]
        fake_audio: cli.fake_audio,
    };
    // O app registra os próprios erros (no `scriba.log`).
    app::run(opts)
}
