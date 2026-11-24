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
        /// Template name (default: general)
        #[arg(short, long)]
        template: Option<String>,
        /// Fill template placeholders from a .toml, .yaml or .yml file
        /// (repeatable; later files win key by key)
        #[arg(long, value_name = "FILE")]
        values: Vec<PathBuf>,
    },
    /// Compile project to PDF
    Build {
        /// Fill template placeholders from a .toml, .yaml or .yml file
        /// (only on a folder with a template.toml; staged to a temp copy)
        /// (repeatable; later files win key by key)
        #[arg(long, value_name = "FILE", conflicts_with = "watch")]
        values: Vec<PathBuf>,
        /// PDF output path (requires --values; default: <folder-name>.pdf
        /// in the parent directory)
        #[arg(long, value_name = "PATH", requires = "values")]
        out: Option<PathBuf>,
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
        /// After each successful rebuild, write a PNG of one page to a fixed
        /// path so any file-watching image viewer becomes a live preview
        /// (requires `--watch`)
        #[arg(long, requires = "watch")]
        preview: bool,
        /// Page to rasterize for `--preview` (1-based; default: 1)
        #[arg(long, default_value_t = 1, requires = "preview")]
        preview_page: usize,
        /// Fixed path for the `--preview` PNG (default: preview.png)
        #[arg(long, value_name = "PATH", requires = "preview")]
        preview_out: Option<PathBuf>,
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
    /// Inspect the compiled PDF (text, info, pages, fidelity)
    Pdf {
        #[command(subcommand)]
        action: PdfAction,
    },
    /// Manage templates
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
    /// Manage the personal spell-check whitelist (dictionary)
    Spell {
        #[command(subcommand)]
        action: SpellAction,
    },
    /// Diagnose the managed environment (Tectonic, cache, fonts, dictionaries, project)
    Doctor,
    /// Print the agent skill embedded in this binary
    Skill {
        /// Also print the bundled reference files
        #[arg(long, conflicts_with = "list")]
        references: bool,
        /// List the bundled reference file names, one per line
        #[arg(long)]
        list: bool,
    },
    /// Update texforge to the latest stable release (always asks first)
    ///
    /// Exit codes: 0 = up to date (or update installed / declined /
    /// cargo-managed refusal), 1 = an update is available (--check mode),
    /// 2 = the update check could not be completed (network, DNS, TLS,
    /// HTTP ≥ 400, or an unparsable response; the cause is printed on
    /// stderr).
    Update {
        /// Only report whether an update is available (exit 0 = up to date, 1 = update available, 2 = check failed)
        #[arg(long)]
        check: bool,
        /// Install the update without asking (the prompt defaults to NO)
        #[arg(long)]
        yes: bool,
    },
    /// Remove everything texforge manages under ~/.texforge
    Uninstall {
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
        /// Print the plan without removing anything
        #[arg(long)]
        dry_run: bool,
        /// Also remove the personal spell dictionary (your own writing)
        #[arg(long)]
        include_spell_words: bool,
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
enum PdfAction {
    /// Print text as a reader or ATS would see it
    Text {
        /// Keep ligature codepoints unnormalized
        #[arg(long)]
        raw: bool,
    },
    /// Report pages, fonts (embedded?), and metadata
    Info,
    /// Report which section opens each page (diff-friendly)
    Pages,
    /// Check significant source words appear in the PDF text
    Check,
}

#[derive(Subcommand)]
enum SpellAction {
    /// Add one or more words to the whitelist (default: global personal dictionary)
    Add {
        /// Word(s) to add
        #[arg(required = true)]
        words: Vec<String>,
        /// Target the project's whitelist instead of the global personal dictionary
        #[arg(long, conflicts_with = "global")]
        local: bool,
        /// Target the global personal dictionary (~/.texforge/spell-words); this is the default
        #[arg(long, conflicts_with = "local")]
        global: bool,
    },
    /// List the effective whitelist words for the scope (default: global personal dictionary)
    List {
        /// List the project's whitelist instead of the global personal dictionary
        #[arg(long, conflicts_with = "global")]
        local: bool,
        /// List the global personal dictionary (~/.texforge/spell-words); this is the default
        #[arg(long, conflicts_with = "local")]
        global: bool,
    },
    /// Remove one or more words from the whitelist (default: global personal dictionary)
    Remove {
        /// Word(s) to remove
        #[arg(required = true)]
        words: Vec<String>,
        /// Target the project's whitelist instead of the global personal dictionary
        #[arg(long, conflicts_with = "global")]
        local: bool,
        /// Target the global personal dictionary (~/.texforge/spell-words); this is the default
        #[arg(long, conflicts_with = "local")]
        global: bool,
    },
}

#[derive(Subcommand)]
enum TemplateAction {
    /// List available templates (installed + every remote source by default)
    List {
        /// Only show locally installed templates (skip the remote sources)
        #[arg(long)]
        local: bool,
        /// Print one JSON array (`name`, `source`, `installed`) and nothing else
        #[arg(long)]
        json: bool,
    },
    /// Add a template from a registry source or from a local folder
    Add {
        /// Template name, `source/name`, or a path to a template folder
        source: String,
        /// Replace a template that is already installed
        #[arg(long)]
        force: bool,
    },
    /// Remove an installed template (name or `source/name`)
    Remove { name: String },
    /// Validate template compatibility
    Validate { name: String },
    /// Refresh cached templates (bypass TTL; all templates or one by name)
    Refresh {
        /// Template name (or `source/name`) to refresh (omit to refresh all)
        name: Option<String>,
    },
    /// Manage extra template sources (registries)
    Source {
        #[command(subcommand)]
        action: SourceAction,
    },
}

#[derive(Subcommand)]
enum SourceAction {
    /// Add a template source from a GitHub repository
    Add {
        /// Local name for the source
        name: String,
        /// GitHub `owner/name`, `https://github.com/owner/name` or `.git` URL
        repo_or_url: String,
        /// Git ref to use (default: main)
        #[arg(long)]
        r#ref: Option<String>,
        /// Skip the reachability check (for offline use)
        #[arg(long)]
        no_check: bool,
    },
    /// List configured template sources (official first)
    List {
        /// Print one JSON array instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Remove a template source
    Remove {
        /// Name of the source to remove
        name: String,
    },
}

impl Cli {
    pub fn execute(self) -> Result<()> {
        match self.command {
            Commands::Clean => commands::clean::execute(),
            Commands::Init => commands::init::execute(),
            Commands::New {
                name,
                template,
                values,
            } => commands::new::execute(&name, template.as_deref(), values.as_slice()),
            Commands::Build {
                values,
                out,
                watch,
                delay,
                verbose,
                reproducible,
                preview,
                preview_page,
                preview_out,
            } => {
                if !values.is_empty() {
                    return commands::build_template::execute(
                        values.as_slice(),
                        out.as_deref(),
                        verbose,
                        reproducible,
                    );
                }
                if watch {
                    let live_preview = preview.then_some(commands::build::LivePreview {
                        page: preview_page,
                        out: preview_out,
                    });
                    commands::build::watch(delay, verbose, reproducible, live_preview.as_ref())
                } else {
                    commands::build::execute(verbose, reproducible)
                }
            }
            Commands::Fmt { check } => commands::fmt::execute(check),
            Commands::Check { deny_warnings } => commands::check::execute(deny_warnings),
            Commands::Stats { json, by } => commands::stats::execute(json, by),
            Commands::Outline { json } => commands::outline::execute(json),
            Commands::Preview { page, scale, out } => commands::preview::execute(page, scale, out),
            Commands::Pdf { action } => {
                let action = match action {
                    PdfAction::Text { raw } => commands::pdf::PdfAction::Text { raw },
                    PdfAction::Info => commands::pdf::PdfAction::Info,
                    PdfAction::Pages => commands::pdf::PdfAction::Pages,
                    PdfAction::Check => commands::pdf::PdfAction::Check,
                };
                commands::pdf::execute(action)
            }
            Commands::Template { action } => execute_template(action),
            Commands::Spell { action } => {
                let action = match action {
                    SpellAction::Add {
                        words,
                        local,
                        global,
                    } => commands::spell::SpellAction::Add {
                        words,
                        scope: commands::spell::Scope::from_flags(local, global),
                    },
                    SpellAction::List { local, global } => commands::spell::SpellAction::List {
                        scope: commands::spell::Scope::from_flags(local, global),
                    },
                    SpellAction::Remove {
                        words,
                        local,
                        global,
                    } => commands::spell::SpellAction::Remove {
                        words,
                        scope: commands::spell::Scope::from_flags(local, global),
                    },
                };
                commands::spell::execute(action)
            }
            Commands::Doctor => commands::doctor::execute(),
            Commands::Skill { references, list } => commands::skill::execute(list, references),
            Commands::Update { check, yes } => {
                // `update` reports status through exit codes — 0 up to date,
                // 1 update available (`--check`), 2 the check itself could not
                // be completed (cause already on stderr) — so a non-zero code
                // is a result, not an error to be wrapped in anyhow and printed
                // as a failure. Exit explicitly instead; a check error never
                // becomes a bare anyhow failure.
                let code = commands::update::run_update(check, yes)?;
                if let Some(code) = update_exit_code(check, code) {
                    std::process::exit(code);
                }
                Ok(())
            }
            Commands::Uninstall {
                yes,
                dry_run,
                include_spell_words,
            } => commands::uninstall::execute(yes, dry_run, include_spell_words),
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

/// Dispatch the `texforge template` subcommands (kept out of [`Cli::execute`]
/// so it stays under the line-count lint).
fn execute_template(action: TemplateAction) -> Result<()> {
    match action {
        TemplateAction::List { local, json } => commands::template::list(!local, json),
        TemplateAction::Add { source, force } => commands::template::add(&source, force),
        TemplateAction::Remove { name } => commands::template::remove(&name),
        TemplateAction::Validate { name } => commands::template::validate(&name),
        TemplateAction::Refresh { name } => commands::template::refresh(name.as_deref()),
        TemplateAction::Source { action } => match action {
            SourceAction::Add {
                name,
                repo_or_url,
                r#ref,
                no_check,
            } => commands::template_source::add(&name, &repo_or_url, r#ref.as_deref(), no_check),
            SourceAction::List { json } => commands::template_source::list(json),
            SourceAction::Remove { name } => commands::template_source::remove(&name),
        },
    }
}

/// The exit decision for `texforge update`: `--check` always exits — `0`
/// means up to date, `1` means an update is available, `2` the check itself
/// failed — while a plain `update` exits only when its code is non-zero
/// (failures after a successful check), so `0` falls through to `Ok(())`.
fn update_exit_code(check: bool, code: i32) -> Option<i32> {
    if check || code != 0 {
        Some(code)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TE14 requirement 6: `--local --global` together must be rejected by
    /// clap, not silently resolved to one or the other.
    #[test]
    fn spell_add_rejects_local_and_global_together() {
        let result =
            Cli::try_parse_from(["texforge", "spell", "add", "docker", "--local", "--global"]);
        assert!(
            result.is_err(),
            "`--local --global` together must be a usage error"
        );
    }

    #[test]
    fn spell_list_rejects_local_and_global_together() {
        let result = Cli::try_parse_from(["texforge", "spell", "list", "--local", "--global"]);
        assert!(
            result.is_err(),
            "`--local --global` together must be a usage error"
        );
    }

    #[test]
    fn spell_remove_rejects_local_and_global_together() {
        let result = Cli::try_parse_from([
            "texforge", "spell", "remove", "docker", "--local", "--global",
        ]);
        assert!(
            result.is_err(),
            "`--local --global` together must be a usage error"
        );
    }

    #[test]
    fn spell_add_accepts_local_alone() {
        let result = Cli::try_parse_from(["texforge", "spell", "add", "docker", "--local"]);
        assert!(result.is_ok());
    }

    #[test]
    fn update_with_no_flags_parses() {
        let cli = Cli::try_parse_from(["texforge", "update"]).unwrap();
        match cli.command {
            Commands::Update { check, yes } => {
                assert!(!check);
                assert!(!yes);
            }
            _ => panic!("`texforge update` must parse into Commands::Update"),
        }
    }

    #[test]
    fn update_check_parses() {
        let cli = Cli::try_parse_from(["texforge", "update", "--check"]).unwrap();
        match cli.command {
            Commands::Update { check, yes } => {
                assert!(check);
                assert!(!yes);
            }
            _ => panic!("`texforge update --check` must parse into Commands::Update"),
        }
    }

    #[test]
    fn update_yes_parses() {
        let cli = Cli::try_parse_from(["texforge", "update", "--yes"]).unwrap();
        match cli.command {
            Commands::Update { check, yes } => {
                assert!(!check);
                assert!(yes);
            }
            _ => panic!("`texforge update --yes` must parse into Commands::Update"),
        }
    }

    /// `texforge new --values` must reach the command with the path intact.
    #[test]
    fn new_accepts_a_values_file() {
        let cli = Cli::try_parse_from([
            "texforge",
            "new",
            "demo",
            "-t",
            "contract",
            "--values",
            "client.toml",
        ])
        .unwrap();
        match cli.command {
            Commands::New {
                name,
                template,
                values,
            } => {
                assert_eq!(name, "demo");
                assert_eq!(template.as_deref(), Some("contract"));
                assert_eq!(values, vec![std::path::PathBuf::from("client.toml")]);
            }
            _ => panic!("`texforge new` must parse into Commands::New"),
        }
    }

    /// `texforge new --values` is repeatable: later files win key by key.
    #[test]
    fn new_accepts_repeated_values_files_in_order() {
        let cli = Cli::try_parse_from([
            "texforge",
            "new",
            "demo",
            "--values",
            "lab.toml",
            "--values",
            "client.toml",
        ])
        .unwrap();
        match cli.command {
            Commands::New { values, .. } => {
                assert_eq!(
                    values,
                    vec![
                        std::path::PathBuf::from("lab.toml"),
                        std::path::PathBuf::from("client.toml"),
                    ]
                );
            }
            _ => panic!("`texforge new` must parse into Commands::New"),
        }
    }

    /// `texforge build --values` is repeatable too.
    #[test]
    fn build_accepts_repeated_values_files_in_order() {
        let cli = Cli::try_parse_from([
            "texforge",
            "build",
            "--values",
            "lab.toml",
            "--values",
            "client.toml",
        ])
        .unwrap();
        match cli.command {
            Commands::Build { values, .. } => {
                assert_eq!(
                    values,
                    vec![
                        std::path::PathBuf::from("lab.toml"),
                        std::path::PathBuf::from("client.toml"),
                    ]
                );
            }
            _ => panic!("`texforge build` must parse into Commands::Build"),
        }
    }

    /// `Cli::execute` must really dispatch: an invalid project name fails
    /// inside `texforge new` long before anything touches the disk.
    #[test]
    fn execute_dispatches_to_the_command_and_surfaces_its_error() {
        let cli = Cli::try_parse_from(["texforge", "new", "bad name"]).unwrap();
        let error = cli.execute().unwrap_err();
        assert!(
            error.to_string().contains("cannot contain spaces"),
            "expected the `new` validation error, got: {error}"
        );
    }

    /// The `texforge update` exit contract: `--check` always exits with its
    /// status code; a plain update exits only for a non-zero code.
    #[test]
    fn update_exit_code_follows_the_documented_contract() {
        assert_eq!(update_exit_code(true, 0), Some(0), "--check up to date");
        assert_eq!(
            update_exit_code(true, 1),
            Some(1),
            "--check update available"
        );
        assert_eq!(update_exit_code(true, 2), Some(2), "--check failed");
        assert_eq!(update_exit_code(false, 0), None, "plain update, done");
        assert_eq!(update_exit_code(false, 1), Some(1), "install phase failed");
        assert_eq!(update_exit_code(false, 2), Some(2), "check failed");
    }
}
