use std::process::ExitCode;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Grava, compacta e transcreve")]
struct Cli {
    /// Transcreve uma amostra embutida em CPU e codifica um clipe curto com x264.
    #[arg(long)]
    self_test: bool,
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
    println!("Scriba {}", env!("CARGO_PKG_VERSION"));
    ExitCode::SUCCESS
}
