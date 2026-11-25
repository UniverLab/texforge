//! CITATION.cff and .zenodo.json stay in sync with Cargo.toml for the first Zenodo DOI.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path)).expect(path)
}

fn cargo_version() -> String {
    let manifest = read("Cargo.toml");
    let value: toml::Value = toml::from_str(&manifest).expect("Cargo.toml parses");
    value["package"]["version"]
        .as_str()
        .expect("package.version is a string")
        .to_owned()
}

fn top_level_keys(citation: &str) -> Vec<String> {
    citation
        .lines()
        .filter_map(|line| {
            if line.starts_with(' ') || line.starts_with('\t') || line.trim().is_empty() {
                return None;
            }
            line.split_once(':').map(|(k, _)| k.trim().to_owned())
        })
        .collect()
}

#[test]
fn citation_version_matches_cargo() {
    let version = cargo_version();
    let citation = read("CITATION.cff");
    assert!(
        citation.contains(&format!("version: {version}")),
        "CITATION.cff must contain `version: {version}`"
    );
}

#[test]
fn orcid_matches_in_both_files() {
    let citation = read("CITATION.cff");
    let zenodo: serde_json::Value =
        serde_json::from_str(&read(".zenodo.json")).expect(".zenodo.json parses");
    let zenodo_orcid = zenodo["creators"][0]["orcid"]
        .as_str()
        .expect("creators[0].orcid is a string");
    let cff_orcid = citation
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("orcid:")
                .map(|rest| rest.trim().to_owned())
        })
        .expect("CITATION.cff has an orcid field");
    assert_eq!(cff_orcid, zenodo_orcid);
}

#[test]
fn zenodo_has_required_keys() {
    let zenodo: serde_json::Value =
        serde_json::from_str(&read(".zenodo.json")).expect(".zenodo.json parses");
    for key in [
        "title",
        "upload_type",
        "description",
        "creators",
        "license",
        "keywords",
        "related_identifiers",
    ] {
        assert!(
            zenodo.get(key).is_some(),
            ".zenodo.json missing key `{key}`"
        );
    }
    assert_eq!(zenodo["upload_type"], "software");
    assert_eq!(zenodo["license"], "MIT");
}

#[test]
fn citation_keys_within_spec_set() {
    let citation = read("CITATION.cff");
    let allowed: HashSet<&str> = [
        "cff-version",
        "title",
        "type",
        "version",
        "date-released",
        "abstract",
        "license",
        "repository-code",
        "url",
        "authors",
        "keywords",
    ]
    .into_iter()
    .collect();
    for key in top_level_keys(&citation) {
        assert!(
            allowed.contains(key.as_str()),
            "CITATION.cff has unexpected top-level key `{key}`"
        );
    }
    assert!(
        citation.contains("date-released: RELEASE_DATE"),
        "CITATION.cff must carry the verbatim `date-released: RELEASE_DATE` placeholder"
    );
}
