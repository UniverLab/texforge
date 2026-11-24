//! Placeholder values loaded from a file for `texforge new --values`.
//!
//! The file is a flat table of placeholder name to scalar. TOML (`.toml`) and
//! YAML (`.yaml`, `.yml`) are accepted; the format follows the extension.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// First line of the `values.toml` written into a generated project.
const VALUES_FILE_HEADER: &str =
    "# Values used by texforge new; run texforge new with --values on this file to regenerate.";

/// Load a values file into placeholder name -> string.
///
/// Strings stay as they are, integers print in decimal, floats use Rust's
/// `f64` `to_string` and booleans become `true` / `false`.
pub fn load(path: &Path) -> Result<BTreeMap<String, String>> {
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_ascii_lowercase);
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read values file '{}'", path.display()))?;
    match extension.as_deref() {
        Some("toml") => load_toml(&text),
        Some("yaml" | "yml") => load_yaml(&text),
        _ => bail!(
            "Values file '{}' must end in .toml, .yaml or .yml",
            path.display()
        ),
    }
}

/// Load several values files in command-line order and merge them key by
/// key: a later file's value replaces an earlier one, absent keys are kept.
/// Each file may be TOML or YAML independently by its own extension.
///
/// Returns the merged map plus, for every key, the file that last set it
/// (used to attribute validation errors to the right source file).
pub fn load_many(
    paths: &[PathBuf],
) -> Result<(BTreeMap<String, String>, HashMap<String, PathBuf>)> {
    let mut merged = BTreeMap::new();
    let mut origins: HashMap<String, PathBuf> = HashMap::new();
    for path in paths {
        let values = load(path)?;
        for (key, value) in values {
            merged.insert(key.clone(), value);
            origins.insert(key, path.to_path_buf());
        }
    }
    Ok((merged, origins))
}

/// Render a `values.toml` for a generated project: the header comment, then
/// one `key = "value"` line per key, sorted and escaped as a TOML basic string.
pub fn render(values: &BTreeMap<String, String>) -> String {
    render_with_sources(values, &[])
}

/// Render the merged `--values` map: like [`render`], but with a second
/// header line listing the source files in the order they were given. With
/// zero or one source the output is exactly [`render`]'s, so a single
/// `--values` file echoes byte-identically to before.
pub fn render_with_sources(values: &BTreeMap<String, String>, sources: &[PathBuf]) -> String {
    let mut out = String::from(VALUES_FILE_HEADER);
    out.push('\n');
    if sources.len() > 1 {
        let listed = sources
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("# Sources: {listed} in order\n"));
    }
    for (key, value) in values {
        out.push_str(&format!("{} = {}\n", toml_key(key), toml_string(value)));
    }
    out
}

/// `key` as a TOML key: bare when it is a valid bare key, quoted otherwise, so
/// a name like `user.name` does not come back as a nested table.
fn toml_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if bare {
        key.to_string()
    } else {
        toml_string(key)
    }
}

/// `value` as a quoted TOML basic string — always `"…"`, never a literal
/// string — with backslashes, quotes and control characters escaped. Shared
/// with `texforge new`, which writes these values into `project.toml`.
pub(crate) fn toml_string(value: &str) -> String {
    format!("\"{}\"", escape_basic_string(value))
}

fn load_toml(text: &str) -> Result<BTreeMap<String, String>> {
    let table: toml::Table = toml::from_str(text).context("Values file is not valid TOML")?;
    let mut values = BTreeMap::new();
    for (key, value) in table {
        values.insert(key.clone(), toml_scalar(&key, &value)?);
    }
    Ok(values)
}

fn toml_scalar(key: &str, value: &toml::Value) -> Result<String> {
    match value {
        toml::Value::String(s) => Ok(s.clone()),
        toml::Value::Integer(i) => Ok(i.to_string()),
        toml::Value::Float(f) => Ok(f.to_string()),
        toml::Value::Boolean(b) => Ok(b.to_string()),
        _ => bail!("Values key '{key}' must be a string, integer, float or boolean"),
    }
}

fn load_yaml(text: &str) -> Result<BTreeMap<String, String>> {
    let document: serde_norway::Value =
        serde_norway::from_str(text).context("Values file is not valid YAML")?;
    let mapping = match document {
        serde_norway::Value::Null => return Ok(BTreeMap::new()),
        serde_norway::Value::Mapping(mapping) => mapping,
        _ => bail!("Values file must be a flat table of keys and scalar values"),
    };
    let mut values = BTreeMap::new();
    for (key, value) in mapping {
        let Some(name) = key.as_str() else {
            bail!("Values key '{}' must be a string", yaml_label(&key));
        };
        values.insert(name.to_string(), yaml_scalar(name, &value)?);
    }
    Ok(values)
}

fn yaml_scalar(key: &str, value: &serde_norway::Value) -> Result<String> {
    match value {
        serde_norway::Value::String(s) => Ok(s.clone()),
        serde_norway::Value::Bool(b) => Ok(b.to_string()),
        serde_norway::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.to_string())
            } else if let Some(u) = n.as_u64() {
                Ok(u.to_string())
            } else if let Some(f) = n.as_f64() {
                Ok(f.to_string())
            } else {
                bail!("Values key '{key}' is not a representable number")
            }
        }
        _ => bail!("Values key '{key}' must be a string, integer, float or boolean"),
    }
}

/// A printable label for a YAML key that is not a string.
fn yaml_label(key: &serde_norway::Value) -> String {
    key.as_str()
        .map_or_else(|| format!("{key:?}"), str::to_string)
}

/// Escape a value so it fits in a TOML basic string: the escapes TOML defines
/// for backslash, quote, newline, carriage return and tab, and `\uXXXX` for
/// any other character TOML forbids raw.
fn escape_basic_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04X}", ch as u32)),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn same_data_in_toml_and_yaml_loads_identically() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = write(
            dir.path(),
            "client.toml",
            "title = \"Contract\"\ncount = 42\nratio = 1.5\ndraft = true\n",
        );
        let yaml_path = write(
            dir.path(),
            "client.yaml",
            "title: Contract\ncount: 42\nratio: 1.5\ndraft: true\n",
        );
        let from_toml = load(&toml_path).unwrap();
        let from_yaml = load(&yaml_path).unwrap();
        assert_eq!(from_toml, from_yaml);
        assert_eq!(from_toml["title"], "Contract");
        assert_eq!(from_toml["count"], "42");
        assert_eq!(from_toml["ratio"], "1.5");
        assert_eq!(from_toml["draft"], "true");
    }

    #[test]
    fn yml_extension_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "client.yml", "author: Ada\n");
        assert_eq!(load(&path).unwrap()["author"], "Ada");
    }

    #[test]
    fn every_scalar_type_formats_as_a_string() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "v.toml",
            "s = \"text\"\ni = -7\nf = 2.25\nb = false\n",
        );
        let values = load(&path).unwrap();
        assert_eq!(values["s"], "text");
        assert_eq!(values["i"], "-7");
        assert_eq!(values["f"], "2.25");
        assert_eq!(values["b"], "false");
    }

    /// An integer stays an integer in decimal even above `i64::MAX`, where a
    /// fallback to `f64` would print it as `1e19`.
    #[test]
    fn yaml_large_integer_prints_in_decimal() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.yaml", "big: 18446744073709551615\n");
        assert_eq!(load(&path).unwrap()["big"], "18446744073709551615");
    }

    #[test]
    fn toml_nested_table_is_rejected_naming_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.toml", "[client]\nname = \"A\"\n");
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("client"), "{error}");
    }

    #[test]
    fn toml_array_is_rejected_naming_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.toml", "tags = [\"a\"]\n");
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("tags"), "{error}");
    }

    #[test]
    fn toml_datetime_is_rejected_naming_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.toml", "issued = 2024-01-02T03:04:05Z\n");
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("issued"), "{error}");
    }

    #[test]
    fn yaml_null_is_rejected_naming_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.yaml", "author: null\n");
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("author"), "{error}");
    }

    #[test]
    fn yaml_nested_mapping_is_rejected_naming_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.yaml", "client:\n  name: A\n");
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("client"), "{error}");
    }

    #[test]
    fn yaml_sequence_is_rejected_naming_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.yaml", "tags:\n  - a\n");
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("tags"), "{error}");
    }

    #[test]
    fn yaml_non_string_key_is_rejected_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.yaml", "1: value\n");
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains('1'), "{error}");
    }

    #[test]
    fn yaml_top_level_sequence_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.yaml", "- a\n- b\n");
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains("flat table"), "{error}");
    }

    #[test]
    fn yaml_empty_document_loads_no_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "v.yaml", "---\n");
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn unsupported_extension_names_the_three_accepted_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "client.json", "{}");
        let error = load(&path).unwrap_err().to_string();
        assert!(error.contains(".toml"), "{error}");
        assert!(error.contains(".yaml"), "{error}");
        assert!(error.contains(".yml"), "{error}");
    }

    #[test]
    fn render_writes_the_header_then_sorted_escaped_lines() {
        let mut values = BTreeMap::new();
        values.insert("z".to_string(), "back\\slash".to_string());
        values.insert("a".to_string(), "say \"hi\"".to_string());
        values.insert("m".to_string(), "tab\there".to_string());

        let text = render(&values);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "# Values used by texforge new; run texforge new with --values on this file to regenerate."
        );
        assert_eq!(lines[1], "a = \"say \\\"hi\\\"\"");
        assert_eq!(lines[2], "m = \"tab\\there\"");
        assert_eq!(lines[3], "z = \"back\\\\slash\"");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn rendered_values_load_back_to_the_same_map() {
        let dir = tempfile::tempdir().unwrap();
        let mut values = BTreeMap::new();
        values.insert("author".to_string(), "A \\ \"B\"".to_string());
        values.insert("title".to_string(), "Contract".to_string());

        let path = write(dir.path(), "values.toml", &render(&values));
        assert_eq!(load(&path).unwrap(), values);
    }

    /// FR6: every value is a TOML basic string, so a LaTeX value loaded from a
    /// file and written back keeps its backslashes and quotes verbatim.
    #[test]
    fn rendered_values_survive_latex_backslashes_and_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let mut values = BTreeMap::new();
        values.insert("author".to_string(), "A \\textbf{B} \\& \"C\"".to_string());
        values.insert("amount".to_string(), "50\\%".to_string());

        let path = write(dir.path(), "values.toml", &render(&values));
        assert_eq!(load(&path).unwrap(), values);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("author = \"A \\\\textbf{B} \\\\& \\\"C\\\"\""),
            "values must be basic strings:\n{written}"
        );
    }

    #[test]
    fn later_files_win_key_by_key_and_origins_name_the_last_setter() {
        let dir = tempfile::tempdir().unwrap();
        let first = write(
            dir.path(),
            "lab.toml",
            "title = \"Lab\"\nauthor = \"Lab Author\"\n",
        );
        let second = write(dir.path(), "client.toml", "title = \"Contract\"\n");
        let (merged, origins) = load_many(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(merged["title"], "Contract");
        assert_eq!(merged["author"], "Lab Author");
        assert_eq!(origins["title"], second);
        assert_eq!(origins["author"], first);
    }

    #[test]
    fn toml_and_yaml_files_mix_with_later_files_winning() {
        let dir = tempfile::tempdir().unwrap();
        let first = write(
            dir.path(),
            "lab.toml",
            "title = \"Lab\"\nlanguage = \"spanish\"\n",
        );
        let second = write(dir.path(), "client.yaml", "title: Contract\n");
        let (merged, origins) = load_many(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(merged["title"], "Contract");
        assert_eq!(merged["language"], "spanish");
        assert_eq!(origins["title"], second);
        assert_eq!(origins["language"], first);
    }

    #[test]
    fn a_key_set_only_by_the_first_file_survives_the_merge() {
        let dir = tempfile::tempdir().unwrap();
        let first = write(dir.path(), "lab.toml", "author = \"Lab Author\"\n");
        let second = write(dir.path(), "client.toml", "title = \"Contract\"\n");
        let (merged, _) = load_many(&[first, second]).unwrap();
        assert_eq!(merged["author"], "Lab Author");
        assert_eq!(merged["title"], "Contract");
    }

    #[test]
    fn missing_second_file_fails_naming_that_file() {
        let dir = tempfile::tempdir().unwrap();
        let first = write(dir.path(), "lab.toml", "title = \"Lab\"\n");
        let missing = dir.path().join("client.toml");
        let error = load_many(&[first, missing.clone()])
            .unwrap_err()
            .to_string();
        assert!(error.contains(&missing.display().to_string()), "{error}");
    }

    #[test]
    fn rendered_sources_list_every_file_in_order() {
        let mut values = BTreeMap::new();
        values.insert("b".to_string(), "2".to_string());
        values.insert("a".to_string(), "1".to_string());
        let text = render_with_sources(
            &values,
            &[PathBuf::from("lab.toml"), PathBuf::from("client.toml")],
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], VALUES_FILE_HEADER);
        assert_eq!(lines[1], "# Sources: lab.toml, client.toml in order");
        assert_eq!(lines[2], "a = \"1\"");
        assert_eq!(lines[3], "b = \"2\"");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn rendered_single_source_matches_the_old_output() {
        let mut values = BTreeMap::new();
        values.insert("title".to_string(), "Contract".to_string());
        assert_eq!(
            render_with_sources(&values, &[PathBuf::from("client.toml")]),
            render(&values)
        );
    }

    /// A dotted placeholder name must stay one flat key: written bare it would
    /// read back as a nested table and be rejected on the next run.
    #[test]
    fn dotted_key_round_trips_as_a_flat_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut values = BTreeMap::new();
        values.insert("user.name".to_string(), "Ada".to_string());

        let path = write(dir.path(), "values.toml", &render(&values));
        assert_eq!(load(&path).unwrap(), values);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("\"user.name\" = \"Ada\""),
            "a dotted key must be quoted:\n{written}"
        );
    }
}
