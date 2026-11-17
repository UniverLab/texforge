//! Contract tests freezing the 1.0 command surface.
//!
//! The checked-in `tests/cli_surface.txt` lists every command path and long
//! flag the binary supports. These tests walk the compiled binary's `--help`
//! output (the `tests/skill.rs` precedent proves clap usage errors exit 2),
//! compare it with the snapshot, prove the reference documents every row,
//! prove the `--json` outputs keep their documented top-level shape on a
//! real fixture project, and pin the exit-code contract. Documentation lists
//! what the code does today; the code always wins.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_texforge");
const SNAPSHOT: &str = include_str!("cli_surface.txt");
const REFERENCE: &str = include_str!("../docs/cli-reference.md");

fn snapshot_rows() -> BTreeSet<String> {
    SNAPSHOT
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn walk_tree(prefix: &[String], set: &mut BTreeSet<String>) {
    let mut args: Vec<String> = prefix.to_vec();
    args.push("--help".to_string());
    let output = Command::new(BIN)
        .args(&args)
        .env("CI", "1")
        .output()
        .expect("spawn texforge --help");
    assert!(
        output.status.success(),
        "`texforge {}` --help failed",
        prefix.join(" ")
    );
    let help = String::from_utf8(output.stdout).expect("help output is UTF-8");
    let mut in_commands = false;
    for line in help.lines() {
        if line.trim() == "Commands:" {
            in_commands = true;
            continue;
        }
        if in_commands {
            if line.trim().is_empty() {
                in_commands = false;
            } else if let Some(name) = command_entry(line) {
                if name != "help" {
                    let mut path = prefix.to_vec();
                    path.push(name);
                    set.insert(format!("command {}", path.join(" ")));
                    walk_tree(&path, set);
                }
            }
            continue;
        }
        if let Some(row) = option_flag(prefix, line) {
            set.insert(row);
        }
    }
}

/// A command entry is indented exactly two spaces; wrapped descriptions are
/// indented further and must not be mistaken for commands.
fn command_entry(line: &str) -> Option<String> {
    let rest = line.strip_prefix("  ")?;
    if rest.starts_with(' ') {
        return None;
    }
    let name = rest.split_whitespace().next()?;
    if name.starts_with('-') {
        return None;
    }
    Some(name.to_string())
}

/// An option line is indented exactly two or six spaces (clap wraps
/// descriptions at ten). Strips an optional `-x, ` short form, requires a
/// long `--name`, and classifies the value kind from the line shape.
fn option_flag(prefix: &[String], line: &str) -> Option<String> {
    let width = line.len() - line.trim_start_matches(' ').len();
    if width != 2 && width != 6 {
        return None;
    }
    let trimmed = line.trim_start();
    if !trimmed.starts_with('-') {
        return None;
    }
    let rest = if trimmed.starts_with("--") {
        trimmed
    } else {
        let comma = trimmed.find(", ")?;
        &trimmed[comma + 2..]
    };
    let end = rest.find([' ', '=', '<', '[']).unwrap_or(rest.len());
    let name = &rest[..end];
    if !name.starts_with("--") || name.len() <= 2 {
        return None;
    }
    if name == "--help" || name == "--version" {
        return None;
    }
    let kind = if line.contains(" [<") {
        "optional"
    } else if line.contains(" <") {
        "value"
    } else {
        "bool"
    };
    let path = prefix.join(" ");
    if path.is_empty() {
        None
    } else {
        Some(format!("flag {path} {name} {kind}"))
    }
}

#[test]
fn surface_is_frozen() {
    let mut walked = BTreeSet::new();
    walk_tree(&[], &mut walked);
    let expected = snapshot_rows();
    let missing: Vec<&String> = expected.difference(&walked).collect();
    let extra: Vec<&String> = walked.difference(&expected).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "the 1.0 command surface is frozen; update tests/cli_surface.txt \
         deliberately with docs/cli-reference.md.\nmissing: {missing:?}\nextra: {extra:?}"
    );
}

/// The reference split by its `### texforge <path>` headings: command path to
/// section body. A section runs until the next `##`/`###` heading; `####`
/// subsections stay inside their command's body.
fn doc_sections() -> BTreeMap<String, String> {
    let mut sections = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for line in REFERENCE.lines() {
        if let Some(path) = line.strip_prefix("### texforge ") {
            if let Some((path, body)) = current.take() {
                sections.insert(path, body);
            }
            current = Some((path.trim().to_string(), String::new()));
        } else if line.starts_with("## ") {
            if let Some((path, body)) = current.take() {
                sections.insert(path, body);
            }
        } else if let Some((_, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some((path, body)) = current.take() {
        sections.insert(path, body);
    }
    sections
}

#[test]
fn snapshot_is_documented() {
    let sections = doc_sections();
    for row in snapshot_rows() {
        let mut parts = row.split_whitespace();
        match parts.next() {
            Some("command") => {
                let path = parts.collect::<Vec<&str>>().join(" ");
                let body = sections.get(&path).unwrap_or_else(|| {
                    panic!("docs/cli-reference.md has no `### texforge {path}` section")
                });
                assert!(
                    !body.trim().is_empty(),
                    "docs/cli-reference.md's `### texforge {path}` section is empty"
                );
            }
            Some("flag") => {
                let mut items: Vec<&str> = parts.collect();
                let _kind = items.pop().expect("flag row names a value kind");
                let name = items.pop().expect("flag row names a flag");
                let path = items.join(" ");
                let body = sections.get(&path).unwrap_or_else(|| {
                    panic!("docs/cli-reference.md has no `### texforge {path}` section")
                });
                let row_prefix = format!("| `{name}`");
                assert!(
                    body.lines()
                        .any(|line| line.trim_start().starts_with(&row_prefix)),
                    "docs/cli-reference.md's `texforge {path}` flag table never lists `{name}`"
                );
            }
            other => panic!("bad snapshot row: {other:?} in `{row}`"),
        }
    }
}

/// Minimal fixture project: a title, an author, the offline `general`
/// template, and one section of body text.
fn fixture_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("fixture tempdir");
    std::fs::write(
        dir.path().join("project.toml"),
        "[document]\ntitle = \"T\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
    )
    .expect("write project.toml");
    std::fs::write(
        dir.path().join("main.tex"),
        "\\begin{document}\n\\section{Intro}\nbody words here\n\\end{document}\n",
    )
    .expect("write main.tex");
    dir
}

fn run_on_fixture(dir: &tempfile::TempDir, args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .current_dir(dir.path())
        .env("CI", "1")
        .output()
        .expect("spawn texforge")
}

fn json_stdout(output: &std::process::Output, args: &str) -> serde_json::Value {
    assert!(
        output.status.success(),
        "`texforge {args}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8");
    serde_json::from_str(&text).expect("stdout is JSON")
}

/// The documented top-level field table for a command's `--json` output:
/// field name to documented type.
fn documented_json_fields(command: &str) -> BTreeMap<String, String> {
    let heading = format!("#### `texforge {command} --json` fields");
    let mut lines = REFERENCE.lines();
    assert!(
        lines.by_ref().any(|line| line.trim() == heading),
        "docs/cli-reference.md has no `{heading}` subsection"
    );
    let mut fields = BTreeMap::new();
    let mut in_table = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "Top-level object:" {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if trimmed.is_empty() {
            if fields.is_empty() {
                continue;
            }
            break;
        }
        if !trimmed.starts_with('|') {
            break;
        }
        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() < 2 || cells[0] == "Field" || cells[0].starts_with("---") {
            continue;
        }
        fields.insert(cells[0].trim_matches('`').to_string(), cells[1].to_string());
    }
    assert!(
        !fields.is_empty(),
        "docs/cli-reference.md documents no top-level fields for `texforge {command} --json`"
    );
    fields
}

fn assert_field_type(field: &str, ty: &str, value: &serde_json::Value) {
    let matches = match ty {
        "string" => value.is_string(),
        "integer" => value.is_u64(),
        "array" => value.is_array(),
        other => panic!(
            "docs/cli-reference.md gives `{field}` the type `{other}`; teach this test about it"
        ),
    };
    assert!(
        matches,
        "`{field}` must be a {ty} per docs/cli-reference.md, got {value}"
    );
}

/// Assert the parsed output carries exactly the top-level fields the reference
/// documents, each with its documented type.
fn assert_json_contract(value: &serde_json::Value, command: &str) {
    let object = value.as_object().expect("top level is an object");
    let documented = documented_json_fields(command);
    let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = documented.keys().map(String::as_str).collect();
    assert_eq!(
        actual, expected,
        "`texforge {command} --json` top-level fields drifted from docs/cli-reference.md"
    );
    for (field, ty) in &documented {
        let field_value = object
            .get(field)
            .unwrap_or_else(|| panic!("`{field}` is documented but missing from the output"));
        assert_field_type(field, ty, field_value);
    }
}

#[test]
fn stats_json_contract() {
    let dir = fixture_project();
    let output = run_on_fixture(&dir, &["stats", "--json"]);
    let value = json_stdout(&output, "stats --json");
    assert_json_contract(&value, "stats");
    assert_eq!(
        value["document"].as_str(),
        Some("T"),
        "document must be the project title string"
    );
    assert!(!value["sections"]
        .as_array()
        .expect("sections is documented as an array")
        .is_empty());
}

#[test]
fn outline_json_contract() {
    let dir = fixture_project();
    let output = run_on_fixture(&dir, &["outline", "--json"]);
    let value = json_stdout(&output, "outline --json");
    assert_json_contract(&value, "outline");
    assert_eq!(
        value["document"].as_str(),
        Some("T"),
        "document must be the project title string"
    );
    assert!(
        !value["sections"]
            .as_array()
            .expect("sections is documented as an array")
            .is_empty(),
        "the fixture has one section"
    );
}

#[test]
fn exit_codes() {
    let dir = fixture_project();
    let stats = run_on_fixture(&dir, &["stats"]);
    assert_eq!(stats.status.code(), Some(0), "stats on a fixture exits 0");

    let mut missing = PathBuf::from(dir.path());
    missing.push("broken.tex");
    std::fs::write(
        &missing,
        "\\begin{document}\n\\section{Intro}\n\\input{missing}\n\\end{document}\n",
    )
    .expect("write broken.tex");
    std::fs::write(
        dir.path().join("project.toml"),
        "[document]\ntitle = \"T\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"broken.tex\"\n",
    )
    .expect("point the entry at broken.tex");
    let check = run_on_fixture(&dir, &["check"]);
    assert_eq!(
        check.status.code(),
        Some(1),
        "check with a missing input exits 1, got {:?}: {}",
        check.status.code(),
        String::from_utf8_lossy(&check.stderr)
    );

    let usage = run_on_fixture(&dir, &["stats", "--by", "bogus"]);
    assert_eq!(
        usage.status.code(),
        Some(2),
        "a bad --by value is a usage error and exits 2"
    );
}
