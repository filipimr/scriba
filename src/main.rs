use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use scriba::app::{self, Options};
use scriba::config::Paths;

#[derive(Parser)]
#[command(version, about = "Grava, compacta e transcreve")]
struct Cli {
    /// Transcreve uma amostra embutida em CPU e codifica um clipe curto com x264.
    #[arg(long)]
    self_test: bool,

    /// Não abre o navegador ao iniciar.
    #[arg(long)]
    no_open: bool,

    /// Porta local preferida; se estiver ocupada, usa outra livre. 0 = qualquer uma.
    #[arg(long, default_value_t = 47800)]
    port: u16,

    /// Dados internos do app (padrão: %LOCALAPPDATA%\Scriba).
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// Pasta das saídas (padrão: Documentos\Scriba).
    #[arg(long)]
    output_dir: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.self_test {
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
        paths: Paths::resolve(cli.data_dir, cli.output_dir),
    };
    match app::run(opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e:#}");
            ExitCode::FAILURE
        }
    }
}
