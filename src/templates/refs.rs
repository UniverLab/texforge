//! Template references: `name` or `source/name`, resolved against the
//! population of templates a command accepts.
//!
//! A bare `name` offered by exactly one source resolves to it; offered by
//! more than one, it is a clash the user must disambiguate with
//! `source/name`. There is no carve-out for the official source.

use anyhow::Result;

use super::sources::{is_valid_name, OFFICIAL_NAME};

/// One template offering: the source that offers it and the template name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Source name (`official` for the official registry).
    pub source: String,
    /// Template name within that source.
    pub name: String,
}

impl Candidate {
    /// The form commands accept: bare for the official source (so older
    /// projects keep building), `source/name` otherwise.
    pub fn reference(&self) -> String {
        if self.source == OFFICIAL_NAME {
            self.name.clone()
        } else {
            self.qualified()
        }
    }

    /// Always `source/name`, used in clash listings.
    pub fn qualified(&self) -> String {
        format!("{}/{}", self.source, self.name)
    }
}

/// Split a template reference into an optional source and the template name.
///
/// A single `/` splits: the part before must be a valid source name. More
/// than one `/`, or an empty side, is not a reference at all.
pub fn parse_ref(input: &str) -> Result<(Option<String>, String)> {
    match input.split_once('/') {
        None => Ok((None, input.to_string())),
        Some((source, name)) => {
            if source.is_empty() || name.is_empty() || name.contains('/') {
                anyhow::bail!(
                    "'{input}' is neither a local folder nor a registry template name. \
                     Use a registry name (texforge template list) or a path to a template folder."
                );
            }
            if !is_valid_name(source) {
                anyhow::bail!(
                    "'{input}' is neither a local folder nor a registry template name. \
                     Use a registry name (texforge template list) or a path to a template folder."
                );
            }
            Ok((Some(source.to_string()), name.to_string()))
        }
    }
}

/// Resolve `reference` (bare or `source/name`) against `candidates`.
///
/// `source_names` lists the valid sources (official first), used to explain
/// an unknown source part. A bare name offered by more than one source fails
/// listing every candidate as `source/name`.
pub fn resolve_in(
    reference: &str,
    candidates: &[Candidate],
    source_names: &[String],
) -> Result<Candidate> {
    let (wanted_source, wanted_name) = parse_ref(reference)?;
    if let Some(source) = wanted_source {
        if !source_names.iter().any(|s| s == &source) {
            anyhow::bail!(
                "Unknown template source '{source}'. Valid sources: {}",
                source_names.join(", ")
            );
        }
        return candidates
            .iter()
            .find(|c| c.source == source && c.name == wanted_name)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Template '{wanted_name}' not found in source '{source}'. \
                     Run 'texforge template list' to see what it offers."
                )
            });
    }
    let mut matching: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| c.name == wanted_name)
        .collect();
    matching.sort_by(|a, b| a.source.cmp(&b.source));
    match matching.as_slice() {
        [] => anyhow::bail!(
            "Template '{wanted_name}' not found. \
             Run 'texforge template list' to see available templates."
        ),
        [only] => Ok((*only).clone()),
        _ => {
            let lines = matching
                .iter()
                .map(|c| format!("  - {}", c.qualified()))
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!(
                "Template '{wanted_name}' is offered by more than one source:\n{lines}\n\
                 Pick one with `source/name`."
            )
        }
    }
}

/// Installed templates as candidates, from both storage layouts.
pub fn installed_candidates() -> Result<Vec<Candidate>> {
    let mut candidates: Vec<Candidate> = super::list_cached_entries()?
        .into_iter()
        .map(|e| Candidate {
            source: e.source,
            name: e.name,
        })
        .collect();
    candidates.sort_by(|a, b| (&a.name, &a.source).cmp(&(&b.name, &b.source)));
    candidates.dedup_by(|a, b| a.name == b.name && a.source == b.source);
    Ok(candidates)
}

/// One unreachable source: its name and why fetching it failed.
pub struct SourceFailure {
    /// Source name, as in `Warning: source '<name>': <reason>`.
    pub source: String,
    /// First line of the fetch error.
    pub reason: String,
}

/// The population for `new -t`, `init -t` and `template add`: installed
/// templates plus the remote listing of every configured source. An
/// unreachable source contributes nothing; its failure is returned alongside
/// so the caller can warn once per source.
pub struct Union {
    /// Installed and remote templates, sorted and deduplicated.
    pub candidates: Vec<Candidate>,
    /// One entry per source that could not be fetched.
    pub failures: Vec<SourceFailure>,
}

/// Build the [`Union`] population for name resolution.
pub fn union_candidates() -> Result<Union> {
    let mut candidates = installed_candidates()?;
    let mut failures = Vec::new();
    for source in super::load_sources()? {
        match super::list_remote_from(&source) {
            Ok(names) => candidates.extend(names.into_iter().map(|name| Candidate {
                source: source.name.clone(),
                name,
            })),
            Err(e) => failures.push(SourceFailure {
                source: source.name.clone(),
                reason: trimmed_error(&e),
            }),
        }
    }
    candidates.sort_by(|a, b| (&a.name, &a.source).cmp(&(&b.name, &b.source)));
    candidates.dedup();
    Ok(Union {
        candidates,
        failures,
    })
}

/// First line of an error, without a trailing period pile-up in warnings.
fn trimmed_error(e: &anyhow::Error) -> String {
    format!("{e:#}")
        .lines()
        .next()
        .unwrap_or("unknown error")
        .to_string()
}

/// The one line printed for an unreachable source (stdout in human `list`,
/// stderr everywhere else): `Warning: source '<name>': <reason>`.
pub fn warning_line(source: &str, reason: &str) -> String {
    format!("Warning: source '{source}': {reason}")
}
