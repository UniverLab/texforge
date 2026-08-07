//! texforge — Self-contained LaTeX to PDF compiler CLI.
//!
//! A command-line tool that compiles LaTeX documents to PDF without requiring
//! TeX Live, `MiKTeX`, or any external LaTeX distribution.

mod cli;
mod commands;
mod compiler;
mod config;
mod diagrams;
mod domain;
mod formatter;
mod linter;
mod manifest;
mod placeholders;
mod raster;
mod templates;
mod texparse;
mod texutil;
mod utils;
mod version;
mod version_checker;
mod wordcount;

use anyhow::Result;
use clap::Parser;

use cli::Cli;

fn main() -> Result<()> {
    // reqwest is built with `rustls-no-provider`, so install the ring crypto
    // provider as the process default before any HTTPS request is made.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();
    cli.execute()
}
