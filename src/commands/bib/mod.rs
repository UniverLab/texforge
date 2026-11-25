//! `texforge bib` command implementation.

pub mod http;
pub mod model;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::domain::project::Project;
use http::{BibHttpClient, LookupOutcome};

/// Subcommands under `texforge bib`.
pub enum BibAction {
    Check {
        file: Option<PathBuf>,
        offline: bool,
        no_cache: bool,
        deny_warnings: bool,
        json: bool,
    },
}

/// Options controlling `check_bib_file`.
pub struct CheckOptions {
    pub offline: bool,
    pub no_cache: bool,
    pub deny_warnings: bool,
    pub json: bool,
}

/// Verification outcome for one identifier (or one un-identified entry in `--offline`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BibCheckResult {
    pub key: String,
    pub id: String,
    pub kind: String,
    pub status: String,
    pub reason: String,
    pub expected_title: Option<String>,
    pub found_title: Option<String>,
}

/// Execute a `texforge bib` subcommand.
pub fn execute(action: BibAction) -> Result<()> {
    match action {
        BibAction::Check {
            file,
            offline,
            no_cache,
            deny_warnings,
            json,
        } => {
            let bib_path = resolve_bib_path(file.as_deref())?;
            let cache_dir = crate::utils::cache_dir()?.join("bib");
            let opts = CheckOptions {
                offline,
                no_cache,
                deny_warnings,
                json,
            };
            let results = check_bib_file(&bib_path, &cache_dir, &opts)?;
            if opts.json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else {
                print_human_report(&results, opts.offline);
            }
            enforce_exit_policy(&results, opts.deny_warnings)
        }
    }
}

fn resolve_bib_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let project = Project::load()?;
    let rel = project
        .config
        .build
        .bibliography
        .as_deref()
        .ok_or_else(|| {
            anyhow::anyhow!("No bibliography configured in project.toml ([build] bibliography)")
        })?;
    Ok(project.root.join(rel))
}

/// Check every DOI and arXiv identifier in `bib_path` and return structured results.
pub fn check_bib_file(
    bib_path: &Path,
    cache_dir: &Path,
    opts: &CheckOptions,
) -> Result<Vec<BibCheckResult>> {
    let source = std::fs::read_to_string(bib_path)
        .with_context(|| format!("Failed to read bibliography file: {}", bib_path.display()))?;
    let records = model::parse_bib_entries(&source);

    if opts.offline {
        return Ok(check_offline(&records));
    }

    let mut client = BibHttpClient::new(cache_dir.to_path_buf(), opts.no_cache)?;
    #[cfg(test)]
    {
        client = client.with_min_interval(std::time::Duration::ZERO);
    }

    let mut results = Vec::new();
    for record in &records {
        let arxiv_id = model::extract_arxiv_id(record);
        let doi = record
            .doi
            .as_deref()
            .and_then(model::normalize_doi)
            .filter(|d| !model::is_arxiv_doi(d));

        if let Some(d) = doi {
            let outcome = client.check_doi(&d)?;
            results.push(evaluate_outcome(record, &d, "doi", outcome));
        }
        if let Some(id) = arxiv_id {
            let outcome = client.check_arxiv(&id)?;
            results.push(evaluate_outcome(record, &id, "arxiv", outcome));
        }
    }

    Ok(results)
}

fn check_offline(records: &[model::BibRecord]) -> Vec<BibCheckResult> {
    let mut results = Vec::new();
    for record in records {
        let has_arxiv = model::extract_arxiv_id(record).is_some();
        let has_doi = record
            .doi
            .as_deref()
            .and_then(model::normalize_doi)
            .is_some();
        if !has_arxiv && !has_doi {
            results.push(BibCheckResult {
                key: record.key.clone(),
                id: String::new(),
                kind: String::new(),
                status: "info".to_string(),
                reason: "no identifier".to_string(),
                expected_title: record.title.clone(),
                found_title: None,
            });
        }
    }
    results
}

fn evaluate_outcome(
    record: &model::BibRecord,
    id: &str,
    kind: &str,
    outcome: LookupOutcome,
) -> BibCheckResult {
    match outcome {
        LookupOutcome::Missing => BibCheckResult {
            key: record.key.clone(),
            id: id.to_string(),
            kind: kind.to_string(),
            status: "error".to_string(),
            reason: "missing".to_string(),
            expected_title: record.title.clone(),
            found_title: None,
        },
        LookupOutcome::Found {
            title: found_title,
            year: found_year,
        } => {
            let Some(expected) = record.title.as_deref() else {
                return BibCheckResult {
                    key: record.key.clone(),
                    id: id.to_string(),
                    kind: kind.to_string(),
                    status: "warning".to_string(),
                    reason: "missing title in .bib".to_string(),
                    expected_title: None,
                    found_title: Some(found_title),
                };
            };
            if !model::title_matches(expected, &found_title) {
                return BibCheckResult {
                    key: record.key.clone(),
                    id: id.to_string(),
                    kind: kind.to_string(),
                    status: "error".to_string(),
                    reason: "different work".to_string(),
                    expected_title: Some(expected.to_string()),
                    found_title: Some(found_title),
                };
            }
            if kind == "doi" {
                if let Some((bib_y, cr_y)) = model::year_drift(record.year.as_deref(), found_year) {
                    return BibCheckResult {
                        key: record.key.clone(),
                        id: id.to_string(),
                        kind: kind.to_string(),
                        status: "warning".to_string(),
                        reason: format!("year mismatch (bib: {bib_y}, crossref: {cr_y})"),
                        expected_title: Some(expected.to_string()),
                        found_title: Some(found_title),
                    };
                }
            }
            BibCheckResult {
                key: record.key.clone(),
                id: id.to_string(),
                kind: kind.to_string(),
                status: "ok".to_string(),
                reason: "ok".to_string(),
                expected_title: Some(expected.to_string()),
                found_title: Some(found_title),
            }
        }
    }
}

fn print_human_report(results: &[BibCheckResult], offline: bool) {
    if offline {
        for r in results {
            println!("  info     {} — {}", r.key, r.reason);
        }
        println!(
            "Summary: {} entry(ies) without identifier (offline)",
            results.len()
        );
        return;
    }

    let mut ok_count = 0usize;
    let mut warn_count = 0usize;
    let mut err_count = 0usize;

    for r in results {
        match r.status.as_str() {
            "ok" => {
                ok_count += 1;
                println!("  ok       {} ({}: {})", r.key, r.kind, r.id);
            }
            "warning" => {
                warn_count += 1;
                println!("  warning  {} ({}: {}) — {}", r.key, r.kind, r.id, r.reason);
            }
            _ => {
                err_count += 1;
                println!("  error    {} ({}: {}) — {}", r.key, r.kind, r.id, r.reason);
                if r.reason == "different work" {
                    println!(
                        "           expected: {}",
                        r.expected_title.as_deref().unwrap_or("")
                    );
                    println!(
                        "           found:    {}",
                        r.found_title.as_deref().unwrap_or("")
                    );
                }
            }
        }
    }

    println!(
        "Summary: {} checked, {} ok, {} warning(s), {} error(s)",
        results.len(),
        ok_count,
        warn_count,
        err_count
    );
}

fn enforce_exit_policy(results: &[BibCheckResult], deny_warnings: bool) -> Result<()> {
    let errors = results.iter().filter(|r| r.status == "error").count();
    let warnings = results.iter().filter(|r| r.status == "warning").count();
    if errors > 0 || (deny_warnings && warnings > 0) {
        anyhow::bail!("bibliography check failed ({errors} error(s), {warnings} warning(s))");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct MockServer {
        base_url: String,
        hits: Arc<AtomicUsize>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl MockServer {
        fn start<F>(handler: F) -> Self
        where
            F: Fn(&str) -> (u16, &'static str, String) + Send + Sync + 'static,
        {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let hits = Arc::new(AtomicUsize::new(0));
            let hits_clone = Arc::clone(&hits);
            let handler = Arc::new(handler);

            listener.set_nonblocking(false).unwrap();
            let handle = std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut stream = stream;
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let first_line = req.lines().next().unwrap_or("");
                    if first_line.starts_with("GET /__stop") {
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                        break;
                    }
                    hits_clone.fetch_add(1, Ordering::SeqCst);
                    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
                    let (code, ctype, body) = handler(path);
                    let reason = match code {
                        200 => "OK",
                        404 => "Not Found",
                        429 => "Too Many Requests",
                        _ => "Error",
                    };
                    let resp = format!(
                        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
            });

            Self {
                base_url: format!("http://{addr}"),
                hits,
                handle: Some(handle),
            }
        }
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            if let Ok(mut s) =
                std::net::TcpStream::connect(self.base_url.trim_start_matches("http://"))
            {
                let _ = s.write_all(b"GET /__stop HTTP/1.1\r\nHost: localhost\r\n\r\n");
            }
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    fn write_temp_bib(dir: &Path, content: &str) -> PathBuf {
        let p = dir.join("refs.bib");
        std::fs::write(&p, content).unwrap();
        p
    }

    fn default_opts() -> CheckOptions {
        CheckOptions {
            offline: false,
            no_cache: false,
            deny_warnings: false,
            json: false,
        }
    }

    #[test]
    fn matching_title_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let bib = write_temp_bib(
            tmp.path(),
            r#"@article{smith2023,
  title = {Deep Learning for {LaTeX} Documents},
  year = {2023},
  doi = {10.1000/ok},
}"#,
        );
        let server = MockServer::start(|_| {
            (
                200,
                "application/json",
                r#"{"message":{"title":["Deep Learning for LaTeX Documents"],"issued":{"date-parts":[[2023]]}}}"#.into(),
            )
        });
        http::set_test_bases(Some(&server.base_url), None);
        let res = check_bib_file(&bib, &tmp.path().join("cache"), &default_opts()).unwrap();
        http::clear_test_bases();

        assert_eq!(res.len(), 1);
        assert_eq!(res[0].status, "ok");
        assert_eq!(res[0].key, "smith2023");
        assert!(enforce_exit_policy(&res, false).is_ok());
    }

    #[test]
    fn mismatched_title_different_work() {
        let tmp = tempfile::tempdir().unwrap();
        let bib = write_temp_bib(
            tmp.path(),
            r#"@article{bad2023,
  title = {Quantum Gravity},
  year = {2023},
  doi = {https://doi.org/10.1000/wrong},
}"#,
        );
        let server = MockServer::start(|_| {
            (
                200,
                "application/json",
                r#"{"message":{"title":["Cooking with Pasta"],"issued":{"date-parts":[[2023]]}}}"#
                    .into(),
            )
        });
        http::set_test_bases(Some(&server.base_url), None);
        let res = check_bib_file(&bib, &tmp.path().join("cache"), &default_opts()).unwrap();
        http::clear_test_bases();

        assert_eq!(res[0].status, "error");
        assert_eq!(res[0].reason, "different work");
        assert_eq!(res[0].expected_title.as_deref(), Some("Quantum Gravity"));
        assert_eq!(res[0].found_title.as_deref(), Some("Cooking with Pasta"));
        assert!(enforce_exit_policy(&res, false).is_err());
    }

    #[test]
    fn doi_404_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let bib = write_temp_bib(
            tmp.path(),
            r#"@article{gone,
  title = {Ghost Paper},
  doi = {doi:10.1000/missing},
}"#,
        );
        let server = MockServer::start(|_| (404, "text/plain", "Not Found".into()));
        http::set_test_bases(Some(&server.base_url), None);
        let res = check_bib_file(&bib, &tmp.path().join("cache"), &default_opts()).unwrap();
        http::clear_test_bases();

        assert_eq!(res[0].status, "error");
        assert_eq!(res[0].reason, "missing");
        assert!(enforce_exit_policy(&res, false).is_err());
    }

    #[test]
    fn year_drift_warning_and_deny_warnings_exit() {
        let tmp = tempfile::tempdir().unwrap();
        let bib = write_temp_bib(
            tmp.path(),
            r#"@article{drift,
  title = {Some Paper},
  year = {2019},
  doi = {10.1000/drift},
}"#,
        );
        let server = MockServer::start(|_| {
            (
                200,
                "application/json",
                r#"{"message":{"title":["Some Paper"],"issued":{"date-parts":[[2023]]}}}"#.into(),
            )
        });
        http::set_test_bases(Some(&server.base_url), None);
        let res = check_bib_file(&bib, &tmp.path().join("cache"), &default_opts()).unwrap();
        http::clear_test_bases();

        assert_eq!(res[0].status, "warning");
        assert!(res[0].reason.contains("year"));
        assert!(enforce_exit_policy(&res, false).is_ok());
        assert!(enforce_exit_policy(&res, true).is_err());
    }

    #[test]
    fn arxiv_missing_empty_feed() {
        let tmp = tempfile::tempdir().unwrap();
        let bib = write_temp_bib(
            tmp.path(),
            r#"@misc{ax1,
  title = {Missing Preprint},
  eprint = {9999.99999},
  archivePrefix = {arXiv},
}"#,
        );
        let server = MockServer::start(|_| {
            (
                200,
                "application/atom+xml",
                r#"<?xml version="1.0"?><feed><title>ArXiv Query</title></feed>"#.into(),
            )
        });
        http::set_test_bases(None, Some(&server.base_url));
        let res = check_bib_file(&bib, &tmp.path().join("cache"), &default_opts()).unwrap();
        http::clear_test_bases();

        assert_eq!(res[0].kind, "arxiv");
        assert_eq!(res[0].status, "error");
        assert_eq!(res[0].reason, "missing");
    }

    #[test]
    fn cache_hit_second_run_skips_network() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let bib = write_temp_bib(
            tmp.path(),
            r#"@article{cached,
  title = {Cached Paper},
  doi = {10.1000/cached},
}"#,
        );
        let server = MockServer::start(|_| {
            (
                200,
                "application/json",
                r#"{"message":{"title":["Cached Paper"],"issued":{"date-parts":[[2024]]}}}"#.into(),
            )
        });
        http::set_test_bases(Some(&server.base_url), None);
        let _ = check_bib_file(&bib, &cache, &default_opts()).unwrap();
        assert_eq!(server.hits.load(Ordering::SeqCst), 1);

        // Second run hits cache
        let _ = check_bib_file(&bib, &cache, &default_opts()).unwrap();
        assert_eq!(server.hits.load(Ordering::SeqCst), 1);

        // --no-cache bypasses cache
        let mut no_cache_opts = default_opts();
        no_cache_opts.no_cache = true;
        let _ = check_bib_file(&bib, &cache, &no_cache_opts).unwrap();
        assert_eq!(server.hits.load(Ordering::SeqCst), 2);
        http::clear_test_bases();
    }

    #[test]
    fn offline_reports_no_identifier_only() {
        let tmp = tempfile::tempdir().unwrap();
        let bib = write_temp_bib(
            tmp.path(),
            r#"@article{with_doi,
  title = {Has DOI},
  doi = {10.1000/yes},
}
@book{no_id,
  title = {Plain Book},
  year = {2020},
}"#,
        );
        let mut opts = default_opts();
        opts.offline = true;
        let res = check_bib_file(&bib, &tmp.path().join("cache"), &opts).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].key, "no_id");
        assert_eq!(res[0].reason, "no identifier");
        assert!(enforce_exit_policy(&res, true).is_ok());
    }

    #[test]
    fn json_shape_has_expected_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let bib = write_temp_bib(
            tmp.path(),
            r#"@misc{ax2,
  title = {Attention Is All You Need},
  doi = {10.48550/arXiv.1706.03762},
}"#,
        );
        let server = MockServer::start(|_| {
            (
                200,
                "application/atom+xml",
                r#"<feed><title>ArXiv Query</title><entry><title>Attention Is All You Need</title></entry></feed>"#.into(),
            )
        });
        http::set_test_bases(None, Some(&server.base_url));
        let res = check_bib_file(&bib, &tmp.path().join("cache"), &default_opts()).unwrap();
        http::clear_test_bases();

        let json_str = serde_json::to_string(&res).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let obj = arr[0].as_object().unwrap();
        for field in [
            "key",
            "id",
            "kind",
            "status",
            "reason",
            "expected_title",
            "found_title",
        ] {
            assert!(obj.contains_key(field), "missing JSON field {field}");
        }
        assert_eq!(obj["kind"], "arxiv");
        assert_eq!(obj["status"], "ok");
    }
}
