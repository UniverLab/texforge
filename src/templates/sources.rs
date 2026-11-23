//! Template sources: where templates are downloaded from.
//!
//! The official [`UniverLab/texforge-templates`](https://github.com/UniverLab/texforge-templates)
//! registry is always present and first. Users may add their own template
//! repositories in `~/.texforge/config.toml` (or
//! `$XDG_CONFIG_HOME/texforge/config.toml`) as `[[template_sources]]` tables:
//!
//! ```toml
//! [[template_sources]]
//! name = "acme"
//! repo = "acme/texforge-templates"
//! ref = "main"
//! ```
//!
//! `ref` is optional and defaults to `main`. The name `official` is reserved
//! for the official registry and is never read from the file.

use anyhow::{Context, Result};

/// Reserved name of the official template registry.
pub const OFFICIAL_NAME: &str = "official";
/// `owner/name` of the official template registry.
pub const OFFICIAL_REPO: &str = "UniverLab/texforge-templates";
/// Git ref of the official template registry.
pub const OFFICIAL_REF: &str = "main";

/// A repository templates are downloaded from: a name local to the config,
/// a GitHub `owner/name` repo and a git ref (branch, tag or commit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSource {
    pub name: String,
    pub repo: String,
    pub git_ref: String,
}

impl TemplateSource {
    /// The official registry: always present, first and the default.
    pub fn official() -> Self {
        Self {
            name: OFFICIAL_NAME.to_string(),
            repo: OFFICIAL_REPO.to_string(),
            git_ref: OFFICIAL_REF.to_string(),
        }
    }

    /// True only for the official registry, which never gets a credential.
    pub fn is_official(&self) -> bool {
        self.name == OFFICIAL_NAME
    }

    /// Tarball archive URL this source downloads templates from.
    pub fn tarball_url(&self) -> String {
        format!(
            "https://codeload.github.com/{}/tar.gz/refs/heads/{}",
            self.repo, self.git_ref
        )
    }

    /// URL of the `registry.toml` listing this source's templates.
    pub fn registry_url(&self) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/registry.toml",
            self.repo, self.git_ref
        )
    }
}

/// The official source first, then the user-configured sources in file order.
///
/// A missing config file — or one without a `[[template_sources]]` table —
/// yields only the official source. A configured entry named `official`, a
/// badly shaped name or repo, an empty `ref` or a duplicate name is an error
/// naming the entry and the rule, never a silent skip.
///
/// Not called yet: the `template source` subcommands and listing by source are
/// the next specs. Kept alive until then.
#[allow(dead_code)]
pub fn load_sources() -> Result<Vec<TemplateSource>> {
    let mut sources = vec![TemplateSource::official()];
    let path = crate::config::config_file_path()?;
    let content = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(sources),
        Err(e) => {
            return Err(e)
                .with_context(|| format!("Failed to read config file: {}", path.display()))
        }
        Ok(content) => content,
    };
    let value: toml::Value = toml::from_str(&content).context("Failed to parse config TOML")?;
    let entries = match value.get("template_sources") {
        None => return Ok(sources),
        Some(table) => match table.as_array() {
            Some(entries) => entries,
            None => anyhow::bail!(
                "Invalid `template_sources`: must be an array of tables (`[[template_sources]]`)"
            ),
        },
    };
    for (index, entry) in entries.iter().enumerate() {
        sources.push(parse_entry(index, entry)?);
        let name = &sources.last().expect("just pushed").name;
        if sources[..sources.len() - 1].iter().any(|s| &s.name == name) {
            anyhow::bail!(
                "Invalid `template_sources[{index}]` ('{name}'): duplicate source name '{name}'"
            );
        }
    }
    Ok(sources)
}

/// Parse and validate one `[[template_sources]]` entry.
fn parse_entry(index: usize, entry: &toml::Value) -> Result<TemplateSource> {
    let entry = entry.as_table().ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid `template_sources[{index}]`: must be a table with `name` and `repo`"
        )
    })?;
    let name = entry
        .get("name")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    if name == OFFICIAL_NAME {
        anyhow::bail!(
            "Invalid `template_sources[{index}]` ('{name}'): name '{OFFICIAL_NAME}' is reserved for the official registry"
        );
    }
    if !is_valid_name(name) {
        anyhow::bail!(
            "Invalid `template_sources[{index}]` ('{name}'): invalid name '{name}': must match [a-z0-9][a-z0-9-]*"
        );
    }
    let repo = entry
        .get("repo")
        .and_then(toml::Value::as_str)
        .unwrap_or_default();
    if !is_valid_repo(repo) {
        anyhow::bail!(
            "Invalid `template_sources[{index}]` ('{name}'): invalid repo '{repo}': must be `owner/name`"
        );
    }
    let git_ref = match entry.get("ref") {
        None => OFFICIAL_REF.to_string(),
        Some(value) => {
            let git_ref = value.as_str().unwrap_or_default();
            if git_ref.is_empty() {
                anyhow::bail!(
                    "Invalid `template_sources[{index}]` ('{name}'): invalid ref: must be non-empty (default '{OFFICIAL_REF}')"
                );
            }
            git_ref.to_string()
        }
    };
    Ok(TemplateSource {
        name: name.to_string(),
        repo: repo.to_string(),
        git_ref,
    })
}

/// Lowercase alphanumeric start, then lowercase alphanumeric or `-`.
fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => (),
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Exactly one `/` with non-empty sides and no whitespace.
fn is_valid_repo(repo: &str) -> bool {
    if repo.chars().any(char::is_whitespace) {
        return false;
    }
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) => !owner.is_empty() && !name.is_empty(),
        _ => false,
    }
}

/// The user's own GitHub credential for non-official sources, when one
/// exists: `GITHUB_TOKEN` first, then the output of `gh auth token`.
///
/// A missing, failing or empty `gh` counts as anonymous. The token is never
/// printed or logged; it only travels as an `Authorization` header.
pub(crate) fn github_token() -> Option<String> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.trim().is_empty() {
            return Some(token.trim().to_string());
        }
    }
    let output = std::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

/// A GET request for `url`: anonymous for the official source, carrying the
/// user's own GitHub credential for any other source when one exists.
pub(crate) fn authed_get(url: &str, source: &TemplateSource) -> reqwest::blocking::RequestBuilder {
    let request = reqwest::blocking::Client::new()
        .get(url)
        .header("User-Agent", "texforge");
    if source.is_official() {
        request
    } else if let Some(token) = github_token() {
        request.bearer_auth(token)
    } else {
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hold [`crate::test_sync::ENV_LOCK`] while swapping `HOME`,
    /// `XDG_CONFIG_HOME`, `GITHUB_TOKEN` and `PATH`, restoring all on drop.
    struct Env {
        _lock: std::sync::MutexGuard<'static, ()>,
        _restore: crate::test_sync::EnvGuard,
        tmp: tempfile::TempDir,
    }

    impl Env {
        fn capture() -> Self {
            let lock = crate::test_sync::ENV_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let restore = crate::test_sync::EnvGuard::capture(&[
                "HOME",
                "XDG_CONFIG_HOME",
                "GITHUB_TOKEN",
                "PATH",
            ]);
            let tmp = tempfile::tempdir().unwrap();
            std::env::set_var("HOME", tmp.path());
            std::env::set_var("XDG_CONFIG_HOME", tmp.path());
            Self {
                _lock: lock,
                _restore: restore,
                tmp,
            }
        }

        fn write_config(&self, contents: &str) {
            let dir = self.tmp.path().join("texforge");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("config.toml"), contents).unwrap();
        }

        /// Put a fake `gh` printing `stdout` on `PATH`.
        fn fake_gh(&self, stdout: &str) {
            let dir = self.tmp.path().join("bin");
            std::fs::create_dir_all(&dir).unwrap();
            let script = dir.join("gh");
            std::fs::write(&script, format!("#!/bin/sh\nprintf '%s' '{stdout}'\n")).unwrap();
            set_executable(&script);
            let path = format!("{}:{}", dir.display(), std::env::var("PATH").unwrap());
            std::env::set_var("PATH", path);
        }

        /// A `PATH` with no `gh` on it, so the lookup is anonymous.
        fn path_without_gh(&self) {
            let dir = self.tmp.path().join("empty-bin");
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("PATH", dir.as_os_str());
        }
    }

    #[cfg(unix)]
    fn set_executable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }

    #[cfg(not(unix))]
    fn set_executable(_path: &std::path::Path) {}

    #[test]
    fn missing_config_file_yields_official_only() {
        let _env = Env::capture();
        let sources = load_sources().unwrap();
        assert_eq!(sources, vec![TemplateSource::official()]);
    }

    #[test]
    fn config_without_table_yields_official_only() {
        let env = Env::capture();
        env.write_config("[user]\nname = \"Jane\"\n");
        assert_eq!(load_sources().unwrap(), vec![TemplateSource::official()]);
    }

    #[test]
    fn official_comes_first_then_file_order_with_default_ref() {
        let env = Env::capture();
        env.write_config(
            "[[template_sources]]\nname = \"acme\"\nrepo = \"acme/templates\"\n\n\
             [[template_sources]]\nname = \"lab\"\nrepo = \"lab/tpls\"\nref = \"v2\"\n",
        );
        let sources = load_sources().unwrap();
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0], TemplateSource::official());
        assert_eq!(sources[1].name, "acme");
        assert_eq!(sources[1].git_ref, "main");
        assert_eq!(sources[2].name, "lab");
        assert_eq!(sources[2].git_ref, "v2");
    }

    #[test]
    fn reserved_name_official_is_rejected() {
        let env = Env::capture();
        env.write_config("[[template_sources]]\nname = \"official\"\nrepo = \"x/y\"\n");
        let err = load_sources().expect_err("must fail").to_string();
        assert!(err.contains("template_sources[0]"), "{err}");
        assert!(err.contains("official"), "{err}");
        assert!(err.contains("reserved"), "{err}");
    }

    #[test]
    fn bad_names_are_rejected() {
        for bad in ["Acme", "ac_me", "-lead", "", "has space"] {
            let env = Env::capture();
            env.write_config(&format!(
                "[[template_sources]]\nname = \"{bad}\"\nrepo = \"x/y\"\n"
            ));
            let err = load_sources().expect_err("must fail").to_string();
            assert!(err.contains("template_sources[0]"), "{err}");
            assert!(err.contains("[a-z0-9][a-z0-9-]*"), "{err}");
            drop(env);
        }
    }

    #[test]
    fn bad_repos_are_rejected() {
        for bad in ["noslash", "a/b/c", "/b", "a/", "a/ b", ""] {
            let env = Env::capture();
            env.write_config(&format!(
                "[[template_sources]]\nname = \"ok-name\"\nrepo = \"{bad}\"\n"
            ));
            let err = load_sources().expect_err("must fail").to_string();
            assert!(err.contains("template_sources[0]"), "{err}");
            assert!(err.contains("ok-name"), "{err}");
            assert!(err.contains("owner/name"), "{err}");
            drop(env);
        }
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let env = Env::capture();
        env.write_config(
            "[[template_sources]]\nname = \"acme\"\nrepo = \"a/b\"\n\n\
             [[template_sources]]\nname = \"acme\"\nrepo = \"c/d\"\n",
        );
        let err = load_sources().expect_err("must fail").to_string();
        assert!(err.contains("template_sources[1]"), "{err}");
        assert!(err.contains("acme"), "{err}");
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn empty_ref_is_rejected() {
        let env = Env::capture();
        env.write_config("[[template_sources]]\nname = \"acme\"\nrepo = \"a/b\"\nref = \"\"\n");
        let err = load_sources().expect_err("must fail").to_string();
        assert!(err.contains("template_sources[0]"), "{err}");
        assert!(err.contains("ref"), "{err}");
    }

    #[test]
    fn wrong_type_table_is_rejected() {
        let env = Env::capture();
        env.write_config("template_sources = \"nope\"\n");
        let err = load_sources().expect_err("must fail").to_string();
        assert!(err.contains("template_sources"), "{err}");
    }

    #[test]
    fn official_urls_match_history() {
        let official = TemplateSource::official();
        assert_eq!(official.name, "official");
        assert!(official.is_official());
        assert_eq!(
            official.tarball_url(),
            "https://codeload.github.com/UniverLab/texforge-templates/tar.gz/refs/heads/main"
        );
        assert_eq!(
            official.registry_url(),
            "https://raw.githubusercontent.com/UniverLab/texforge-templates/main/registry.toml"
        );
    }

    #[test]
    fn env_token_wins_over_gh() {
        let env = Env::capture();
        std::env::set_var("GITHUB_TOKEN", "env-secret");
        env.fake_gh("gh-secret");
        assert_eq!(github_token().as_deref(), Some("env-secret"));
    }

    #[test]
    fn gh_output_is_the_fallback() {
        let env = Env::capture();
        std::env::remove_var("GITHUB_TOKEN");
        env.fake_gh("gh-secret");
        assert_eq!(github_token().as_deref(), Some("gh-secret"));
    }

    #[test]
    fn no_credential_is_anonymous() {
        let env = Env::capture();
        std::env::remove_var("GITHUB_TOKEN");
        env.path_without_gh();
        assert_eq!(github_token(), None);
    }

    fn ensure_rustls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    /// Serve one request on 127.0.0.1, forward its raw text, answer 200.
    fn serve_once(sender: std::sync::mpsc::Sender<String>) -> (u16, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap();
            let _ = sender.send(String::from_utf8_lossy(&buf[..n]).to_string());
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        });
        (port, handle)
    }

    #[test]
    fn non_official_request_carries_the_env_token() {
        ensure_rustls();
        let env = Env::capture();
        std::env::set_var("GITHUB_TOKEN", "env-secret-123");
        env.path_without_gh();
        let (tx, rx) = std::sync::mpsc::channel();
        let (port, handle) = serve_once(tx);
        let source = TemplateSource {
            name: "acme".to_string(),
            repo: "acme/private".to_string(),
            git_ref: "main".to_string(),
        };
        let response = authed_get(&format!("http://127.0.0.1:{port}/x"), &source)
            .send()
            .unwrap();
        assert!(response.status().is_success());
        handle.join().unwrap();
        let request = rx.recv().unwrap();
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer env-secret-123"),
            "{request}"
        );
    }

    #[test]
    fn official_request_is_anonymous_despite_env_token() {
        ensure_rustls();
        let env = Env::capture();
        std::env::set_var("GITHUB_TOKEN", "env-secret-123");
        env.path_without_gh();
        let (tx, rx) = std::sync::mpsc::channel();
        let (port, handle) = serve_once(tx);
        let response = authed_get(
            &format!("http://127.0.0.1:{port}/x"),
            &TemplateSource::official(),
        )
        .send()
        .unwrap();
        assert!(response.status().is_success());
        handle.join().unwrap();
        let request = rx.recv().unwrap();
        assert!(
            !request.to_lowercase().contains("authorization:"),
            "{request}"
        );
    }

    #[test]
    fn private_repo_message_names_auth_without_token() {
        let env = Env::capture();
        std::env::set_var("GITHUB_TOKEN", "super-secret-token");
        let message = crate::templates::http::private_repo_message("acme/private");
        assert!(message.contains("acme/private"), "{message}");
        assert!(message.contains("may be private"), "{message}");
        assert!(message.contains("GITHUB_TOKEN"), "{message}");
        assert!(message.contains("gh auth login"), "{message}");
        assert!(!message.contains("super-secret-token"), "{message}");
        drop(env);
    }
}
