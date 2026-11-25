//! Crossref and arXiv HTTP lookups with rate limiting, retry, and 24h cache.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DEFAULT_CROSSREF_BASE: &str = "https://api.crossref.org";
const DEFAULT_ARXIV_BASE: &str = "https://export.arxiv.org";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;
const MAX_RETRY_AFTER_SECS: u64 = 15;

#[cfg(test)]
thread_local! {
    static TEST_CROSSREF_BASE: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
    static TEST_ARXIV_BASE: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_bases(crossref: Option<&str>, arxiv: Option<&str>) {
    TEST_CROSSREF_BASE.with(|b| *b.borrow_mut() = crossref.map(str::to_string));
    TEST_ARXIV_BASE.with(|b| *b.borrow_mut() = arxiv.map(str::to_string));
}

#[cfg(test)]
pub(crate) fn clear_test_bases() {
    TEST_CROSSREF_BASE.with(|b| *b.borrow_mut() = None);
    TEST_ARXIV_BASE.with(|b| *b.borrow_mut() = None);
}

fn crossref_base() -> String {
    #[cfg(test)]
    if let Some(base) = TEST_CROSSREF_BASE.with(|b| b.borrow().clone()) {
        return base;
    }
    DEFAULT_CROSSREF_BASE.to_string()
}

fn arxiv_base() -> String {
    #[cfg(test)]
    if let Some(base) = TEST_ARXIV_BASE.with(|b| b.borrow().clone()) {
        return base;
    }
    DEFAULT_ARXIV_BASE.to_string()
}

/// Result of looking up a single DOI or arXiv identifier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LookupOutcome {
    Found { title: String, year: Option<i32> },
    Missing,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedLookup {
    fetched_at: u64,
    outcome: LookupOutcome,
}

/// Sequential HTTP client with per-host 1 req/s rate limiting and 24h disk cache.
pub struct BibHttpClient {
    client: reqwest::blocking::Client,
    cache_dir: PathBuf,
    no_cache: bool,
    min_interval: Duration,
    last_crossref: Option<Instant>,
    last_arxiv: Option<Instant>,
}

impl BibHttpClient {
    pub fn new(cache_dir: PathBuf, no_cache: bool) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let user_agent = format!("texforge/{}", env!("CARGO_PKG_VERSION"));
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(REQUEST_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(user_agent)
            .build()
            .context("Failed to build HTTP client")?;
        Ok(Self {
            client,
            cache_dir,
            no_cache,
            min_interval: Duration::from_secs(1),
            last_crossref: None,
            last_arxiv: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_min_interval(mut self, interval: Duration) -> Self {
        self.min_interval = interval;
        self
    }

    /// Look up a normalized DOI (`10.xxx/...`) against Crossref.
    pub fn check_doi(&mut self, doi: &str) -> Result<LookupOutcome> {
        let cache_key = format!("crossref:{doi}");
        if !self.no_cache {
            if let Some(cached) = read_cache(&self.cache_dir, &cache_key) {
                return Ok(cached);
            }
        }

        let base = crossref_base();
        let url = format!(
            "{}/works/{}",
            base.trim_end_matches('/'),
            percent_encode_component(doi)
        );
        throttle(&mut self.last_crossref, self.min_interval);
        let response = self.get_with_retry(&url)?;
        let status = response.status();
        let outcome = if status.as_u16() == 404 {
            LookupOutcome::Missing
        } else if status.is_success() {
            let body = response
                .text()
                .context("Failed to read Crossref response body")?;
            parse_crossref_json(&body)?
        } else {
            anyhow::bail!("Crossref returned HTTP {status} for {doi}");
        };

        let _ = write_cache(&self.cache_dir, &cache_key, &outcome);
        Ok(outcome)
    }

    /// Look up an arXiv identifier against the arXiv Atom API.
    pub fn check_arxiv(&mut self, id: &str) -> Result<LookupOutcome> {
        let cache_key = format!("arxiv:{id}");
        if !self.no_cache {
            if let Some(cached) = read_cache(&self.cache_dir, &cache_key) {
                return Ok(cached);
            }
        }

        let base = arxiv_base();
        let url = format!(
            "{}/api/query?id_list={}",
            base.trim_end_matches('/'),
            percent_encode_component(id)
        );
        throttle(&mut self.last_arxiv, self.min_interval);
        let response = self.get_with_retry(&url)?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("arXiv returned HTTP {status} for {id}");
        }
        let body = response
            .text()
            .context("Failed to read arXiv response body")?;
        let outcome = parse_arxiv_atom(&body);
        let _ = write_cache(&self.cache_dir, &cache_key, &outcome);
        Ok(outcome)
    }

    fn get_with_retry(&self, url: &str) -> Result<reqwest::blocking::Response> {
        let resp = self
            .client
            .get(url)
            .send()
            .with_context(|| format!("HTTP request failed for {url}"))?;
        let status = resp.status();
        if status.as_u16() == 429 || status.is_server_error() {
            let wait = parse_retry_after(resp.headers().get("retry-after"));
            std::thread::sleep(wait);
            return self
                .client
                .get(url)
                .send()
                .with_context(|| format!("HTTP retry failed for {url}"));
        }
        Ok(resp)
    }
}

fn throttle(last: &mut Option<Instant>, min_interval: Duration) {
    if !min_interval.is_zero() {
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < min_interval {
                std::thread::sleep(min_interval - elapsed);
            }
        }
    }
    *last = Some(Instant::now());
}

fn parse_retry_after(header: Option<&reqwest::header::HeaderValue>) -> Duration {
    let secs = header
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(1)
        .min(MAX_RETRY_AFTER_SECS);
    Duration::from_secs(secs)
}

fn parse_crossref_json(body: &str) -> Result<LookupOutcome> {
    let val: serde_json::Value =
        serde_json::from_str(body).context("Failed to parse Crossref JSON")?;
    let msg = val
        .get("message")
        .context("Crossref JSON missing 'message' object")?;
    let title = msg
        .get("title")
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let year = msg
        .get("issued")
        .and_then(|i| i.get("date-parts"))
        .and_then(|dp| dp.as_array())
        .and_then(|outer| outer.first())
        .and_then(|inner| inner.as_array())
        .and_then(|parts| parts.first())
        .and_then(|y| y.as_i64())
        .and_then(|y| i32::try_from(y).ok());

    if title.is_empty() {
        Ok(LookupOutcome::Missing)
    } else {
        Ok(LookupOutcome::Found { title, year })
    }
}

fn parse_arxiv_atom(xml: &str) -> LookupOutcome {
    let Some(entry_start) = xml.find("<entry") else {
        return LookupOutcome::Missing;
    };
    let Some(entry_end) = xml[entry_start..].find("</entry>") else {
        return LookupOutcome::Missing;
    };
    let entry = &xml[entry_start..entry_start + entry_end];
    let Some(title_tag) = entry.find("<title") else {
        return LookupOutcome::Missing;
    };
    let Some(after_gt) = entry[title_tag..].find('>') else {
        return LookupOutcome::Missing;
    };
    let content_start = title_tag + after_gt + 1;
    let Some(title_end) = entry[content_start..].find("</title>") else {
        return LookupOutcome::Missing;
    };
    let raw_title = &entry[content_start..content_start + title_end];
    let unescaped = unescape_xml(raw_title);
    let collapsed = unescaped.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        LookupOutcome::Missing
    } else {
        LookupOutcome::Found {
            title: collapsed,
            year: None,
        }
    }
}

fn unescape_xml(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
}

fn percent_encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn cache_path(cache_dir: &Path, key: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    cache_dir.join(format!("{hex}.json"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_cache(cache_dir: &Path, key: &str) -> Option<LookupOutcome> {
    let path = cache_path(cache_dir, key);
    let content = std::fs::read_to_string(path).ok()?;
    let cached: CachedLookup = serde_json::from_str(&content).ok()?;
    let age = now_unix().saturating_sub(cached.fetched_at);
    if age < CACHE_TTL_SECS {
        Some(cached.outcome)
    } else {
        None
    }
}

fn write_cache(cache_dir: &Path, key: &str, outcome: &LookupOutcome) -> Result<()> {
    std::fs::create_dir_all(cache_dir)?;
    let entry = CachedLookup {
        fetched_at: now_unix(),
        outcome: outcome.clone(),
    };
    let path = cache_path(cache_dir, key);
    std::fs::write(path, serde_json::to_string(&entry)?)?;
    Ok(())
}
