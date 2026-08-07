//! CLI argument parsing and command dispatch.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands;

/// Self-contained LaTeX to PDF compiler
#[derive(Parser)]
#[command(name = "texforge", version, about)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Remove build artifacts
    Clean,
    /// Initialize a texforge project in the current directory
    Init,
    /// Create a new project from a template
    New {
        /// Project name
        name: String,
        /// Template name (default: basic)
        #[arg(short, long)]
        template: Option<String>,
    },
    /// Compile project to PDF
    Build {
        /// Watch for file changes and rebuild automatically
        #[arg(long)]
        watch: bool,
        /// Debounce delay in seconds before rebuilding (default: 10)
        #[arg(long, default_value = "2")]
        delay: u64,
        /// Print every engine warning instead of a per-file summary
        #[arg(long)]
        verbose: bool,
        /// Build reproducibly: pin `SOURCE_DATE_EPOCH` so identical source plus
        /// the same Tectonic version yields an identical PDF. An optional epoch
        /// (seconds since the Unix epoch) overrides the fixed default.
        #[arg(long, num_args = 0..=1)]
        reproducible: Option<Option<u64>>,
    },
    /// Format .tex files
    Fmt {
        /// Check formatting without modifying files
        #[arg(long)]
        check: bool,
    },
    /// Lint project without compiling
    Check {
        /// Treat warnings as errors (exit non-zero if any warning is present)
        #[arg(long)]
        deny_warnings: bool,
    },
    /// Count words per section or file
    Stats {
        /// Output JSON instead of a human-readable breakdown
        #[arg(long)]
        json: bool,
        /// Break down by .tex file instead of by section
        #[arg(long, value_enum, default_value = "section")]
        by: crate::commands::stats::ByMode,
    },
    /// Print the document's section tree
    Outline {
        /// Output JSON instead of a human-readable tree
        #[arg(long)]
        json: bool,
    },
    /// Rasterize the compiled PDF to PNG pages
    Preview {
        /// Page number to rasterize (1-based; default: all pages)
        #[arg(long, value_name = "N")]
        page: Option<usize>,
        /// Rasterization scale in pixels per PDF point (default: 1.0)
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        /// Directory to write PNGs to (default: <project>/preview)
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Manage templates
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
    /// Manage global configuration
    Config {
        /// Key to get/set (name, email, institution, language)
        key: Option<String>,
        /// Value to set (optional - if omitted, shows current value)
        value: Option<String>,
    },
}

#[derive(Subcommand)]
enum TemplateAction {
    /// List available templates (installed + remote registry by default)
    List {
        /// Only show locally installed templates (skip the remote registry)
        #[arg(long)]
        local: bool,
    },
    /// Add a template from URL or registry
    Add { source: String },
    /// Remove a template
    Remove { name: String },
    /// Validate template compatibility
    Validate { name: String },
}

impl Cli {
    pub fn execute(self) -> Result<()> {
        match self.command {
            Commands::Clean => commands::clean::execute(),
            Commands::Init => commands::init::execute(),
            Commands::New { name, template } => commands::new::execute(&name, template.as_deref()),
            Commands::Build {
                watch,
                delay,
                verbose,
                reproducible,
            } => {
                if watch {
                    commands::build::watch(delay, verbose, reproducible)
                } else {
                    commands::build::execute(verbose, reproducible)
                }
            }
            Commands::Fmt { check } => commands::fmt::execute(check),
            Commands::Check { deny_warnings } => commands::check::execute(deny_warnings),
            Commands::Stats { json, by } => commands::stats::execute(json, by),
            Commands::Outline { json } => commands::outline::execute(json),
            Commands::Preview { page, scale, out } => commands::preview::execute(page, scale, out),
            Commands::Template { action } => match action {
                TemplateAction::List { local } => commands::template::list(!local),
                TemplateAction::Add { source } => commands::template::add(&source),
                TemplateAction::Remove { name } => commands::template::remove(&name),
                TemplateAction::Validate { name } => commands::template::validate(&name),
            },
            Commands::Config { key, value } => match (key, value) {
                (None, None) => commands::config::wizard(),
                (Some(k), None) if k == "list" => commands::config::list(),
                (Some(k), None) => commands::config::get(&k),
                (Some(k), Some(v)) => commands::config::set(&k, &v),
                (None, Some(_)) => anyhow::bail!("Cannot set value without a key"),
            },
        }
    }
}
