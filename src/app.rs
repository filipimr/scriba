//! Ciclo de vida do app: instância única, servidor local e navegador.

use std::io::Write;

use anyhow::Context;

use crate::config::Paths;
use crate::instance::Instance;
use crate::web::{self, AppState};

pub struct Options {
    pub port: u16,
    pub open_browser: bool,
    pub paths: Paths,
}

/// Sobe o servidor e bloqueia até Ctrl+C. Se outra instância já responde, só reabre o
/// navegador nela.
pub fn run(opts: Options) -> anyhow::Result<()> {
    let file = opts.paths.instance_file();
    std::fs::create_dir_all(&opts.paths.data_dir)
        .with_context(|| format!("criando {}", opts.paths.data_dir.display()))?;

    if let Some(existing) = Instance::read(&file).filter(Instance::is_alive) {
        announce(
            "Scriba já está aberto em",
            &existing.url(),
            opts.open_browser,
        );
        return Ok(());
    }

    let listener = web::bind(opts.port).context("abrindo a porta local")?;
    let instance = Instance::new(listener.local_addr()?.port());
    instance.write(&file)?;
    let state = AppState::new(
        instance.token.clone(),
        instance.id.clone(),
        instance.port,
        opts.paths,
    );
    announce("Scriba em", &instance.url(), opts.open_browser);

    let result = tokio::runtime::Runtime::new()?.block_on(web::serve(listener, state, async {
        let _ = tokio::signal::ctrl_c().await;
    }));
    instance.remove(&file);
    result.context("servidor")
}

fn announce(prefix: &str, url: &str, open_browser: bool) {
    println!("{prefix} {url}");
    let _ = std::io::stdout().flush();
    if open_browser && let Err(e) = open::that_detached(url) {
        eprintln!("não consegui abrir o navegador: {e}");
    }
}
