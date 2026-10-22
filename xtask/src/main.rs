mod contrast;
mod css;
mod design;
mod dist;
mod fixtures;
mod icon;
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
        /// Baixa só os modelos cujo nome de arquivo contém um destes textos, separados por
        /// vírgula (ex.: `base` ou `base,silero`).
        #[arg(long)]
        only: Option<String>,
    },
    /// Gera as fixtures de fala pt-BR em tests/fixtures/ com a voz Maria.
    Fixtures {
        /// Gera só estas fixtures, por nome de arquivo separado por vírgula (ex.:
        /// `fala.mkv,pausas.wav`); as derivadas reaproveitam o `fala.wav` existente.
        #[arg(long)]
        only: Option<String>,
    },
    /// Gera ui/tokens.css do DESIGN-scriba.md e compila assets/app.css com o Tailwind.
    Css {
        /// Só confere se os arquivos versionados estão em dia (CI).
        #[arg(long)]
        check: bool,
    },
    /// Gera os .ico do exe e da bandeja em assets/brand/ a partir de docs/brand/*.svg.
    Icon,
    /// O build da release (o mesmo do release.yml): dist/scriba.exe com Vulkan, o large e o
    /// small embutidos, o SHA-256, as licenças, a conferência das DLLs e o --self-test.
    Dist {
        /// Não roda o --self-test no fim.
        #[arg(long)]
        skip_self_test: bool,
        /// Não limpa o whisper.cpp quando as flags mudam (o CI separa o cache pelas flags).
        #[arg(long)]
        trust_cache: bool,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Model { only } => models::download(&root().join("models"), only.as_deref()),
        Cmd::Fixtures { only } => {
            fixtures::generate(&root().join("tests").join("fixtures"), only.as_deref())
        }
        Cmd::Css { check } => css::run(&root(), check),
        Cmd::Icon => icon::run(&root()),
        Cmd::Dist {
            skip_self_test,
            trust_cache,
        } => dist::run(
            &root(),
            &dist::Options {
                skip_self_test,
                trust_cache,
            },
        ),
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
