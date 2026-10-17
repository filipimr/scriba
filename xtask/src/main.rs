mod contrast;
mod css;
mod design;
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
    /// Gera ui/tokens.css do DESIGN-scriba.md e compila assets/app.css com o Tailwind.
    Css {
        /// Só confere se os arquivos versionados estão em dia (CI).
        #[arg(long)]
        check: bool,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Model { only } => models::download(&root().join("models"), only.as_deref()),
        Cmd::Fixtures => fixtures::generate(&root().join("tests").join("fixtures")),
        Cmd::Css { check } => css::run(&root(), check),
    }
}

/// Raiz do workspace (pai do diretório do xtask).
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask fica dentro do workspace")
        .to_path_buf()
}

#[cfg(test)]
fn design_source() -> std::io::Result<String> {
    std::fs::read_to_string(root().join("DESIGN-scriba.md"))
}
