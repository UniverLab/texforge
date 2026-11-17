//! texforge — Self-contained LaTeX to PDF compiler CLI.
//!
//! A command-line tool that compiles LaTeX documents to PDF without requiring
//! TeX Live, `MiKTeX`, or any external LaTeX distribution.

mod banner;
mod cli;
mod commands;
mod compiler;
mod config;
mod diagrams;
mod domain;
mod formatter;
mod highlight;
mod linter;
mod manifest;
mod pdftext;
mod placeholders;
mod raster;
mod templates;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod test_sync;
mod texparse;
mod texutil;
mod utils;
mod values_file;
mod version;
mod wordcount;

use anyhow::Result;
use clap::Parser;

use cli::Cli;

fn main() -> Result<()> {
    install_closed_pipe_hook();

    // reqwest is built with `rustls-no-provider`, so install the ring crypto
    // provider as the process default before any HTTPS request is made.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();
    cli.execute()
}

/// Turn a panic caused by printing into a closed pipe into a silent exit.
///
/// Every `println!`, `print!` and `eprintln!` in the crate panics when the
/// reader of its pipe has gone away, so `texforge pdf info | head` printed a
/// panic instead of ending quietly. Replacing the hook once, here, covers every
/// print site without changing how any of them reports errors. Every other
/// panic reaches the hook that was installed before this one, unchanged.
fn install_closed_pipe_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if is_closed_pipe_panic(&info.to_string()) {
            std::process::exit(0);
        }
        previous(info);
    }));
}

/// True when a panic message reports a failed write to a stdout or stderr pipe
/// whose reader has gone away.
fn is_closed_pipe_panic(message: &str) -> bool {
    let failed_printing = message.contains("failed printing to stdout")
        || message.contains("failed printing to stderr");
    failed_printing && message.contains("Broken pipe")
}

#[cfg(test)]
mod tests {
    use super::is_closed_pipe_panic;

    #[test]
    fn closed_pipe_panics_are_recognised_for_stdout_and_stderr() {
        assert!(is_closed_pipe_panic(
            "failed printing to stdout: Broken pipe (os error 32)"
        ));
        assert!(is_closed_pipe_panic(
            "failed printing to stderr: Broken pipe (os error 32)"
        ));
    }

    #[test]
    fn other_panics_are_not_mistaken_for_a_closed_pipe() {
        assert!(!is_closed_pipe_panic("index out of bounds"));
        assert!(!is_closed_pipe_panic(
            "failed printing to stdout: No space left on device"
        ));
        assert!(!is_closed_pipe_panic(""));
    }
}
