//! Scriba: grava e transcreve. Por enquanto, a UI, a fila de jobs e o pipeline de
//! transcrição de arquivos (`engine`) com o "Guardar áudio", e o `--self-test` da release.

pub mod app;
pub mod asr;
pub mod clock;
pub mod config;
pub mod dialog;
pub mod engine;
pub mod instance;
pub mod jobs;
pub mod media;
pub mod opener;
pub mod self_test;
pub mod transcript;
pub mod trash;
pub mod web;
