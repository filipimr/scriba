mod fixtures;
mod models;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Tarefas de desenvolvimento do Scriba")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Baixa os modelos para models/ e confere o SHA-256.
    Model {
        /// Baixa só os modelos cujo nome de arquivo contém este texto (ex.: `base`).
        #[arg(long)]
        only: Option<String>,
    },
    /// Gera as fixtures de fala pt-BR em tests/fixtures/ com a voz Maria.
    Fixtures,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Model { only } => models::download(&root().join("models"), only.as_deref()),
        Cmd::Fixtures => fixtures::generate(&root().join("tests").join("fixtures")),
    }
}

/// Raiz do workspace (pai do diretório do xtask).
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask fica dentro do workspace")
        .to_path_buf()
}
