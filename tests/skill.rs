//! End-to-end tests for `texforge skill`: the embedded text, its reference
//! files, and the guarantee that every command and flag the skill documents
//! really exists in the clap command tree.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_texforge");
const SKILL: &str = include_str!("../skill/SKILL.md");

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("spawn the texforge binary")
}

fn stdout_of(args: &[&str]) -> String {
    let output = run(args);
    assert!(
        output.status.success(),
        "`texforge {}` failed with {:?}: {}",
        args.join(" "),
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout must be UTF-8")
}

#[test]
fn skill_prints_the_frontmatter_first_and_exits_zero() {
    let text = stdout_of(&["skill"]);
    assert!(
        text.starts_with("---\n"),
        "stdout must begin with the YAML frontmatter, got:\n{text}"
    );
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("---"));
    assert_eq!(lines.next(), Some("name: texforge"));
    assert!(
        text.contains("\ndescription:"),
        "the frontmatter must declare a description"
    );

    let closing = text
        .lines()
        .skip(1)
        .position(|line| line == "---")
        .expect("frontmatter closing fence")
        + 1;
    let version_line = text
        .lines()
        .position(|line| line.starts_with("This skill was printed by texforge v"))
        .expect("the version line must be present");
    let heading = text
        .lines()
        .position(|line| line.starts_with("# "))
        .expect("the skill body must have a heading");
    assert!(
        closing < version_line,
        "the version line must follow the frontmatter, not precede it"
    );
    assert!(
        version_line < heading,
        "the version line must sit near the top of the body"
    );
    let expected = format!(
        "This skill was printed by texforge v{}",
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        text.lines().any(|line| line == expected),
        "the version must come from CARGO_PKG_VERSION ({expected})"
    );
    for (name, _) in reference_files() {
        assert!(
            !text.contains(&format!("=== references/{name} ===")),
            "plain `skill` must print the skill without appending {name}"
        );
    }
}

fn reference_files() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("skill/references");
    let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.is_file())
        .map(|path| {
            let name = path
                .file_name()
                .expect("file name")
                .to_string_lossy()
                .into_owned();
            let body = std::fs::read_to_string(&path).expect("read reference file");
            (name, body)
        })
        .collect();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(
        !files.is_empty(),
        "ship at least one file under skill/references so --references and --list mean something"
    );
    files
}

#[test]
fn references_prints_every_reference_file_after_the_skill() {
    let references = reference_files();
    let text = stdout_of(&["skill", "--references"]);
    assert!(
        text.starts_with("---\nname: texforge\n"),
        "--references must still lead with the frontmatter"
    );
    assert!(
        text.contains("# texforge CLI"),
        "--references must include the skill text before the files"
    );

    let mut positions = Vec::new();
    for (name, body) in &references {
        let marker = format!("=== references/{name} ===");
        let at = text
            .find(&marker)
            .unwrap_or_else(|| panic!("missing separator for {name}"));
        positions.push(at);
        let rest = &text[at + marker.len()..];
        assert!(
            rest.contains(body.as_str()),
            "the body of {name} must follow its separator"
        );
    }
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "reference files must be printed in name order"
    );
}

#[test]
fn list_prints_exactly_the_reference_names_one_per_line() {
    let expected: Vec<String> = reference_files()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let text = stdout_of(&["skill", "--list"]);
    let actual: Vec<&str> = text.lines().collect();
    assert_eq!(
        actual,
        expected.iter().map(String::as_str).collect::<Vec<_>>(),
        "`skill --list` must print the reference file names, one per line"
    );
}

#[test]
fn skill_contains_contributing_back() {
    let text = stdout_of(&["skill"]);
    assert!(
        text.contains("## Contributing back"),
        "the skill must have a `## Contributing back` section"
    );
    for phrase in [
        "UniverLab/texforge",
        "UniverLab/texforge-templates",
        "{{name}}",
        "texforge template add",
        "texforge template source add",
        "ghscaff",
    ] {
        assert!(
            text.contains(phrase),
            "the skill must mention `{phrase}` in the contributing guidance"
        );
    }
    assert!(
        text.starts_with("---\nname: texforge\n"),
        "the frontmatter must stay unchanged"
    );
}

#[test]
fn references_and_list_cannot_be_combined() {
    let output = run(&["skill", "--references", "--list"]);
    assert!(
        !output.status.success(),
        "`--references --list` together must be rejected"
    );
    assert_eq!(output.status.code(), Some(2), "clap usage errors exit 2");
}

#[derive(Debug, Default)]
struct CommandTree {
    /// Every command path, e.g. `"template add"`.
    commands: BTreeSet<String>,
    /// Every long flag anywhere in the tree.
    flags: BTreeSet<String>,
}

#[test]
fn every_command_and_flag_documented_in_the_skill_exists() {
    let mut tree = CommandTree::default();
    walk_tree(&[], &mut tree);

    let invocations = documented_invocations(SKILL);
    assert!(
        invocations.len() >= 20,
        "the skill should document many commands, found {}",
        invocations.len()
    );

    let mut covered = BTreeSet::new();
    for tokens in invocations {
        let path = resolve(&tree, &tokens);
        assert!(
            !path.is_empty(),
            "SKILL.md documents `texforge {}` but `{}` is not a command in the clap tree",
            tokens.join(" "),
            tokens[0]
        );
        covered.insert(path.join(" "));
        for token in &tokens {
            if let Some(flag) = long_flag(token) {
                assert!(
                    tree.flags.contains(&flag),
                    "SKILL.md documents flag `{flag}` (`texforge {}`) that the CLI does not have",
                    tokens.join(" ")
                );
            }
        }
    }
    assert!(
        covered.len() >= 12,
        "expected the skill to cover many commands, covered {covered:?}"
    );
}

fn walk_tree(prefix: &[String], tree: &mut CommandTree) {
    let mut args: Vec<String> = prefix.to_vec();
    args.push("--help".to_string());
    let output = Command::new(BIN)
        .args(&args)
        .output()
        .expect("spawn texforge --help");
    assert!(
        output.status.success(),
        "`texforge {}--help` failed",
        label(prefix)
    );
    let help = String::from_utf8(output.stdout).expect("help output is UTF-8");
    tree.flags
        .extend(help.split_whitespace().filter_map(long_flag));
    for child in subcommands(&help) {
        if child == "help" {
            continue;
        }
        let mut path = prefix.to_vec();
        path.push(child);
        tree.commands.insert(path.join(" "));
        walk_tree(&path, tree);
    }
}

fn label(prefix: &[String]) -> String {
    if prefix.is_empty() {
        String::new()
    } else {
        format!("{} ", prefix.join(" "))
    }
}

/// The command names listed under clap's `Commands:` heading, in help order.
fn subcommands(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_commands = false;
    for line in help.lines() {
        if line.trim() == "Commands:" {
            in_commands = true;
            continue;
        }
        if !in_commands {
            continue;
        }
        if line.trim().is_empty() {
            break;
        }
        if let Some(name) = command_name(line) {
            names.push(name);
        }
    }
    names
}

/// A command entry is indented exactly two spaces; wrapped descriptions are
/// indented further and must not be mistaken for commands.
fn command_name(line: &str) -> Option<String> {
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

/// `--name[=value]` → `--name`; anything not a long flag (including `---`
/// and LaTeX attributes like `file=`) yields `None`.
fn long_flag(token: &str) -> Option<String> {
    let rest = token.strip_prefix("--")?;
    let first = rest.chars().next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(rest.len());
    Some(format!("--{}", &rest[..end]))
}

/// Every `texforge ...` invocation written in an inline code span or a fenced
/// code block, as its tokens after `texforge`.
fn documented_invocations(skill: &str) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    let mut in_fence = false;
    for line in skill.lines() {
        if line.trim().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        let candidates: Vec<&str> = if in_fence {
            vec![line.trim()]
        } else {
            inline_spans(line)
        };
        for candidate in candidates {
            if let Some(tokens) = invocation_tokens(candidate) {
                found.push(tokens);
            }
        }
    }
    found
}

fn inline_spans(line: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        spans.push(&after[..end]);
        rest = &after[end + 1..];
    }
    spans
}

fn invocation_tokens(text: &str) -> Option<Vec<String>> {
    let text = text.trim();
    let text = text.strip_prefix("$ ").unwrap_or(text);
    let mut tokens = text.split_whitespace();
    if tokens.next() != Some("texforge") {
        return None;
    }
    let rest: Vec<String> = tokens.map(str::to_string).collect();
    match rest.first() {
        Some(first) if !first.starts_with('-') => Some(rest),
        _ => None,
    }
}

/// Greedily descend the documented tokens through the command tree. Arguments
/// stop the descent, so `texforge config name` resolves to `config` and
/// `texforge template add <name>` to `template add`.
fn resolve(tree: &CommandTree, tokens: &[String]) -> Vec<String> {
    let mut path: Vec<String> = Vec::new();
    for token in tokens {
        if token.starts_with('-') || token.starts_with('<') || token.starts_with('[') {
            break;
        }
        let mut candidate = path.clone();
        candidate.push(token.clone());
        if !tree.commands.contains(&candidate.join(" ")) {
            break;
        }
        path = candidate;
    }
    path
}
