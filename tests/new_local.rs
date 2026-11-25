//! `texforge new -t <path>` uses a local template folder in place.
//!
//! The child binary runs with a temporary `HOME`/`XDG_CONFIG_HOME` and an
//! empty git config, so no test can touch the developer's real
//! `~/.texforge`. Every `Output` below comes from a fresh child process: no
//! process-global state is mutated and no lock is needed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_texforge");

/// An isolated run: fresh home (empty texforge config and template store),
/// an empty git config, and a working directory for the child process.
struct Sandbox {
    _root: tempfile::TempDir,
    home: PathBuf,
    work: PathBuf,
    empty_gitconfig: PathBuf,
}

fn sandbox() -> Sandbox {
    let root = tempfile::tempdir().expect("temporary root");
    let home = root.path().join("home");
    let work = root.path().join("work");
    fs::create_dir_all(&home).expect("home dir");
    fs::create_dir_all(&work).expect("work dir");
    let empty_gitconfig = root.path().join("empty.gitconfig");
    fs::write(&empty_gitconfig, "").expect("empty gitconfig");
    Sandbox {
        _root: root,
        home,
        work,
        empty_gitconfig,
    }
}

const BASIC_MANIFEST: &str = r#"id = "local-lab"
version = "0.1.0"
display_name = "Local Lab"
description = "Local fixture template"

[[placeholders]]
name = "title"
type = "string"
description = "Document title"
default = "Local Report"

[[placeholders]]
name = "author"
type = "string"
description = "Author name"
default = "Local Author"
"#;

const REQUIRED_MANIFEST: &str = r#"id = "local-lab"
version = "0.1.0"
display_name = "Local Lab"
description = "Local fixture template"

[[placeholders]]
name = "title"
type = "string"
description = "Document title"
default = "Local Report"

[[placeholders]]
name = "client"
type = "string"
description = "Client legal name"
required = true
"#;

const NO_ID_MANIFEST: &str = r#"version = "0.1.0"
display_name = "Local Lab"
description = "Local fixture template without an id"
"#;

const MAIN_TEX: &str =
    "\\documentclass{article}\n\\title{{title}}\n\\begin{document}\n\\maketitle\n\\end{document}\n";

/// Write a template folder with `manifest` plus a `main.tex` using `{{title}}`.
fn write_template(dir: &Path, manifest: &str) {
    fs::create_dir_all(dir).expect("template dir");
    fs::write(dir.join("template.toml"), manifest).expect("template.toml");
    fs::write(dir.join("main.tex"), MAIN_TEX).expect("main.tex");
}

fn run_new(sandbox: &Sandbox, cwd: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .env("HOME", &sandbox.home)
        .env("XDG_CONFIG_HOME", &sandbox.home)
        .env("GIT_CONFIG_GLOBAL", &sandbox.empty_gitconfig)
        .env("GIT_CONFIG_SYSTEM", &sandbox.empty_gitconfig)
        .env("CI", "1")
        .output()
        .expect("spawn the texforge binary")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Entries under the temporary `~/.texforge/templates/`: a local `-t` path
/// must never install anything there.
fn template_store_entries(sandbox: &Sandbox) -> Vec<PathBuf> {
    let dir = sandbox.home.join(".texforge").join("templates");
    if !dir.is_dir() {
        return Vec::new();
    }
    fs::read_dir(&dir)
        .expect("read template store")
        .map(|entry| entry.expect("store entry").path())
        .collect()
}

fn recorded_template(project: &Path) -> String {
    let text = fs::read_to_string(project.join("project.toml")).expect("project.toml");
    let table: toml::Value = toml::from_str(&text).expect("project.toml parses");
    table["document"]["template"]
        .as_str()
        .expect("document.template is a string")
        .to_owned()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "texforge new failed: {}",
        stderr_of(output)
    );
}

fn assert_no_store_side_effects(sandbox: &Sandbox) {
    assert!(
        template_store_entries(sandbox).is_empty(),
        "a local -t path must not install into ~/.texforge/templates/"
    );
}

#[test]
fn relative_path_uses_the_folder_in_place() {
    let sandbox = &sandbox();
    write_template(&sandbox.work.join("mytpl"), BASIC_MANIFEST);
    let output = run_new(sandbox, &sandbox.work, &["new", "demo", "-t", "./mytpl"]);
    assert_success(&output);
    let project = sandbox.work.join("demo");
    assert_eq!(recorded_template(&project), "local-lab");
    let main = fs::read_to_string(project.join("main.tex")).expect("main.tex");
    assert!(main.contains("\\titledemo"), "title renders:\n{main}");
    assert_no_store_side_effects(sandbox);
}

#[test]
fn absolute_path_works() {
    let sandbox = &sandbox();
    let dir = sandbox.work.join("mytpl");
    write_template(&dir, BASIC_MANIFEST);
    let arg = dir.to_str().expect("utf-8 path").to_owned();
    let output = run_new(sandbox, &sandbox.work, &["new", "demo", "-t", &arg]);
    assert_success(&output);
    assert_eq!(recorded_template(&sandbox.work.join("demo")), "local-lab");
    assert_no_store_side_effects(sandbox);
}

#[test]
fn dot_from_inside_the_template_folder() {
    let sandbox = &sandbox();
    let dir = sandbox.work.join("mytpl");
    write_template(&dir, BASIC_MANIFEST);
    let output = run_new(sandbox, &dir, &["new", "demo", "-t", "."]);
    assert_success(&output);
    assert_eq!(recorded_template(&dir.join("demo")), "local-lab");
    assert_no_store_side_effects(sandbox);
}

#[test]
fn trailing_slash_is_allowed() {
    let sandbox = &sandbox();
    write_template(&sandbox.work.join("mytpl"), BASIC_MANIFEST);
    let output = run_new(sandbox, &sandbox.work, &["new", "demo", "-t", "./mytpl/"]);
    assert_success(&output);
    assert_eq!(recorded_template(&sandbox.work.join("demo")), "local-lab");
    assert_no_store_side_effects(sandbox);
}

#[test]
fn tilde_expands_through_the_home_directory() {
    let sandbox = &sandbox();
    write_template(&sandbox.home.join("tpls").join("mytpl"), BASIC_MANIFEST);
    let output = run_new(
        sandbox,
        &sandbox.work,
        &["new", "demo", "-t", "~/tpls/mytpl"],
    );
    assert_success(&output);
    assert_eq!(recorded_template(&sandbox.work.join("demo")), "local-lab");
    assert_no_store_side_effects(sandbox);
}

#[test]
fn directory_name_may_differ_from_the_manifest_id() {
    let sandbox = &sandbox();
    write_template(&sandbox.work.join("weird-dir"), BASIC_MANIFEST);
    let output = run_new(
        sandbox,
        &sandbox.work,
        &["new", "demo", "-t", "./weird-dir"],
    );
    assert_success(&output);
    let project = sandbox.work.join("demo");
    assert_eq!(
        recorded_template(&project),
        "local-lab",
        "the manifest id is recorded, never the path"
    );
    assert_no_store_side_effects(sandbox);
}

#[test]
fn missing_template_toml_fails_and_creates_no_directory() {
    let sandbox = &sandbox();
    let dir = sandbox.work.join("empty");
    fs::create_dir_all(&dir).expect("empty dir");
    fs::write(dir.join("main.tex"), "x").expect("main.tex");
    let output = run_new(sandbox, &sandbox.work, &["new", "demo", "-t", "./empty"]);
    assert!(!output.status.success(), "missing manifest must fail");
    assert!(
        stderr_of(&output).contains("No template.toml"),
        "the error names what is missing: {}",
        stderr_of(&output)
    );
    assert!(
        !sandbox.work.join("demo").exists(),
        "no project directory may be created"
    );
    assert_no_store_side_effects(sandbox);
}

#[test]
fn manifest_without_id_fails_and_creates_no_directory() {
    let sandbox = &sandbox();
    write_template(&sandbox.work.join("noid"), NO_ID_MANIFEST);
    let output = run_new(sandbox, &sandbox.work, &["new", "demo", "-t", "./noid"]);
    assert!(!output.status.success(), "a manifest without id must fail");
    assert!(
        stderr_of(&output).contains("template.toml"),
        "the error names the manifest file: {}",
        stderr_of(&output)
    );
    assert!(
        !sandbox.work.join("demo").exists(),
        "no project directory may be created"
    );
    assert_no_store_side_effects(sandbox);
}

#[test]
fn missing_main_tex_fails_and_creates_no_directory() {
    let sandbox = &sandbox();
    let dir = sandbox.work.join("nomain");
    fs::create_dir_all(&dir).expect("template dir");
    fs::write(dir.join("template.toml"), BASIC_MANIFEST).expect("template.toml");
    let output = run_new(sandbox, &sandbox.work, &["new", "demo", "-t", "./nomain"]);
    assert!(!output.status.success(), "missing main.tex must fail");
    assert!(
        stderr_of(&output).contains("No main.tex"),
        "the error names what is missing: {}",
        stderr_of(&output)
    );
    assert!(
        !sandbox.work.join("demo").exists(),
        "no project directory may be created"
    );
    assert_no_store_side_effects(sandbox);
}

#[test]
fn bare_directory_name_with_manifest_resolves_as_path() {
    let sandbox = &sandbox();
    write_template(&sandbox.work.join("mytpl"), BASIC_MANIFEST);
    let output = run_new(sandbox, &sandbox.work, &["new", "demo", "-t", "mytpl"]);
    assert_success(&output);
    assert_eq!(recorded_template(&sandbox.work.join("demo")), "local-lab");
    assert_no_store_side_effects(sandbox);
}

#[test]
fn prefixed_path_to_nothing_fails_and_creates_no_directory() {
    let sandbox = &sandbox();
    let output = run_new(
        sandbox,
        &sandbox.work,
        &["new", "demo", "-t", "./does-not-exist"],
    );
    assert!(!output.status.success(), "a path to nothing must fail");
    assert!(
        stderr_of(&output).contains("does-not-exist"),
        "the error names what is missing: {}",
        stderr_of(&output)
    );
    assert!(
        !sandbox.work.join("demo").exists(),
        "no project directory may be created"
    );
    assert_no_store_side_effects(sandbox);
}

#[test]
fn prefixed_path_to_a_file_fails_and_creates_no_directory() {
    let sandbox = &sandbox();
    fs::write(sandbox.work.join("notadir"), "x").expect("regular file");
    let output = run_new(sandbox, &sandbox.work, &["new", "demo", "-t", "./notadir"]);
    assert!(
        !output.status.success(),
        "a path that is not a directory must fail"
    );
    assert!(
        stderr_of(&output).contains("notadir"),
        "the error names what is missing: {}",
        stderr_of(&output)
    );
    assert!(
        !sandbox.work.join("demo").exists(),
        "no project directory may be created"
    );
    assert_no_store_side_effects(sandbox);
}

#[test]
fn unknown_name_still_resolves_as_a_name() {
    let sandbox = &sandbox();
    let output = run_new(
        sandbox,
        &sandbox.work,
        &["new", "demo", "-t", "ghost-name-zzz"],
    );
    assert!(!output.status.success(), "an unknown name must fail");
    let stderr = stderr_of(&output);
    assert!(
        !stderr.contains("No template.toml"),
        "a non-directory name is a name, not a path: {stderr}"
    );
    assert!(
        stderr.contains("ghost-name-zzz"),
        "the error names the template: {stderr}"
    );
    assert!(
        !sandbox.work.join("demo").exists(),
        "no project directory may be created"
    );
}

#[test]
fn values_file_and_required_placeholders_still_apply() {
    let sandbox = &sandbox();
    write_template(&sandbox.work.join("mytpl"), REQUIRED_MANIFEST);
    let missing = run_new(sandbox, &sandbox.work, &["new", "demo", "-t", "./mytpl"]);
    assert!(
        !missing.status.success(),
        "a missing required key must fail"
    );
    let stderr = stderr_of(&missing);
    assert!(stderr.contains("client"), "the key is reported: {stderr}");
    assert!(
        stderr.contains("Client legal name"),
        "the description is reported: {stderr}"
    );
    assert!(
        !sandbox.work.join("demo").exists(),
        "the directory created by this run must be removed"
    );

    let values = sandbox.work.join("client.toml");
    fs::write(&values, "client = \"Acme Ltd\"\n").expect("values file");
    let values_arg = values.to_str().expect("utf-8 path").to_owned();
    let filled = run_new(
        sandbox,
        &sandbox.work,
        &["new", "demo", "-t", "./mytpl", "--values", &values_arg],
    );
    assert_success(&filled);
    let project = sandbox.work.join("demo");
    assert_eq!(recorded_template(&project), "local-lab");
    assert!(
        project.join("values.toml").is_file(),
        "the values file is echoed into the project"
    );
    assert_no_store_side_effects(sandbox);
}

#[test]
fn unknown_values_key_fails_and_creates_no_directory() {
    let sandbox = &sandbox();
    write_template(&sandbox.work.join("mytpl"), BASIC_MANIFEST);
    let values = sandbox.work.join("client.toml");
    fs::write(&values, "client = \"Acme Ltd\"\n").expect("values file");
    let values_arg = values.to_str().expect("utf-8 path").to_owned();
    let output = run_new(
        sandbox,
        &sandbox.work,
        &["new", "demo", "-t", "./mytpl", "--values", &values_arg],
    );
    assert!(!output.status.success(), "an unknown values key must fail");
    assert!(
        stderr_of(&output).contains("client"),
        "the error names the unknown key: {}",
        stderr_of(&output)
    );
    assert!(
        !sandbox.work.join("demo").exists(),
        "no project directory may be created"
    );
    assert_no_store_side_effects(sandbox);
}
