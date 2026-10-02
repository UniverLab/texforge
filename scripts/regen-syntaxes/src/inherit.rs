//! Flatten Sublime Text syntax inheritance (`extends:`) so syntect can load
//! grammars written for Sublime Text 4.
//!
//! syntect 5.3 predates syntax inheritance: it never reads `extends`, so a
//! modern grammar either fails outright (`Context 'main' is missing`, when
//! `main` comes from the parent) or trips over the `meta_prepend` /
//! `meta_append` directives that inheritance uses to order patterns. Upstream
//! `sublimehq/Packages` has used `extends` since 2020 for the languages that
//! matter most here — JavaScript, TypeScript, HTML, LaTeX, SQL, PHP — so
//! loading folders as-is is not an option.
//!
//! The merge below implements exactly what Sublime documents
//! (<https://www.sublimetext.com/docs/syntax.html#inheritance>), nothing more:
//!
//! * only `variables` and `contexts` are inherited; every other top-level key
//!   stays the child's;
//! * variables merge, child wins;
//! * contexts merge by name: a child context replaces the parent's, or — when
//!   it opens with `meta_prepend: true` / `meta_append: true` — its patterns
//!   are spliced before / after the parent's;
//! * a list of parents is processed top to bottom, later parents overriding
//!   earlier ones (they are documented to derive from a common base, so in
//!   practice their contexts are disjoint).
//!
//! Parsing uses `yaml-rust`, the very parser syntect itself uses, so there is
//! no second YAML dialect to disagree with. Only files that actually inherit
//! are re-emitted; everything else passes through byte-for-byte.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use syntect::parsing::SyntaxDefinition;
use yaml_rust::yaml::Hash as YamlHash;
use yaml_rust::{Yaml, YamlEmitter, YamlLoader};

/// Flatten `extends` for `path`, memoising per file so a parent shared by
/// twenty children is only parsed once.
///
/// Returns the flattened YAML text (byte-identical to the file when it does
/// not inherit).
pub(crate) fn flatten(
    path: &Path,
    source_root: &Path,
    cache: &mut HashMap<PathBuf, String>,
) -> Result<String, String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(cached) = cache.get(&canonical) {
        return Ok(cached.clone());
    }
    let mut stack = Vec::new();
    let flattened = flatten_recursive(path, source_root, &mut stack, cache)?;
    cache.insert(canonical, flattened.clone());
    Ok(flattened)
}

/// Load one already-flattened `.sublime-syntax`.
pub(crate) fn load_from_flattened(
    text: &str,
    path: &Path,
    lines_include_newline: bool,
) -> Result<SyntaxDefinition, String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    SyntaxDefinition::load_from_str(text, lines_include_newline, Some(stem))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn key(name: &str) -> Yaml {
    Yaml::String(name.to_string())
}

fn flatten_recursive(
    path: &Path,
    source_root: &Path,
    stack: &mut Vec<PathBuf>,
    cache: &mut HashMap<PathBuf, String>,
) -> Result<String, String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if stack.contains(&canonical) {
        return Err(format!(
            "{}: 'extends' cycle: {}",
            path.display(),
            stack
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" -> ")
        ));
    }
    stack.push(canonical);
    let result = flatten_once(path, source_root, stack, cache);
    stack.pop();
    result
}

fn flatten_once(
    path: &Path,
    source_root: &Path,
    stack: &mut Vec<PathBuf>,
    cache: &mut HashMap<PathBuf, String>,
) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let docs =
        YamlLoader::load_from_str(&text).map_err(|e| format!("{}: YAML: {e}", path.display()))?;
    let Some(Yaml::Hash(child)) = docs.first() else {
        // Empty or non-mapping document — hand it to syntect unchanged so it
        // reports the problem it knows about.
        return Ok(text);
    };

    let inherits = child.contains_key(&key("extends"));
    let has_directives = text.contains("meta_prepend") || text.contains("meta_append");
    if !inherits && !has_directives {
        // The overwhelmingly common case: nothing to merge, byte-identical
        // output.
        return Ok(text);
    }

    let mut merged_contexts = YamlHash::new();
    let mut merged_variables = YamlHash::new();

    if inherits {
        for spec in extend_specs(child.get(&key("extends")))? {
            let parent_path = resolve_parent(&spec, path, source_root)?;
            let parent_text = flatten_recursive(&parent_path, source_root, stack, cache)?;
            let parent = parse_doc(&parent_text, &parent_path)?;
            for (k, v) in table(parent.get(&key("variables"))) {
                merged_variables.insert(k, v);
            }
            for (k, v) in table(parent.get(&key("contexts"))) {
                merged_contexts.insert(k, v);
            }
        }
    }

    // The child's contexts, spliced against what was inherited.
    for (name, value) in table(child.get(&key("contexts"))) {
        let (mode, own) = split_merge_directives(&value);
        let merged = match merged_contexts.get(&name) {
            None => Yaml::Array(own),
            Some(inherited) if mode == MergeMode::Replace => {
                let _ = inherited;
                Yaml::Array(own)
            }
            Some(Yaml::Array(inherited)) => {
                let mut combined = Vec::new();
                match mode {
                    MergeMode::Prepend => {
                        combined.extend(own);
                        combined.extend(inherited.iter().cloned());
                    }
                    MergeMode::Append => {
                        combined.extend(inherited.iter().cloned());
                        combined.extend(own);
                    }
                    MergeMode::Replace => combined.extend(own),
                }
                Yaml::Array(combined)
            }
            Some(_) => Yaml::Array(own),
        };
        merged_contexts.insert(name, merged);
    }

    for (k, v) in table(child.get(&key("variables"))) {
        merged_variables.insert(k, v);
    }

    // Only `variables` and `contexts` are inherited; every other top-level
    // key (`name`, `scope`, `file_extensions`, …) stays the child's own.
    let mut merged = YamlHash::new();
    for (k, v) in child {
        if k == &key("extends") || k == &key("contexts") || k == &key("variables") {
            continue;
        }
        merged.insert(k.clone(), v.clone());
    }
    if !merged_variables.is_empty() {
        merged.insert(key("variables"), Yaml::Hash(merged_variables));
    }
    if !merged_contexts.is_empty() {
        merged.insert(key("contexts"), Yaml::Hash(merged_contexts));
    }

    emit(&Yaml::Hash(merged), path)
}

fn parse_doc(text: &str, path: &Path) -> Result<YamlHash, String> {
    let docs =
        YamlLoader::load_from_str(text).map_err(|e| format!("{}: YAML: {e}", path.display()))?;
    match docs.into_iter().next() {
        Some(Yaml::Hash(map)) => Ok(map),
        other => Err(format!(
            "{}: expected a mapping document, got {}",
            path.display(),
            if other.is_none() {
                "an empty document"
            } else {
                "a non-mapping document"
            }
        )),
    }
}

fn table(value: Option<&Yaml>) -> YamlHash {
    match value {
        Some(Yaml::Hash(map)) => map.clone(),
        _ => YamlHash::new(),
    }
}

fn extend_specs(value: Option<&Yaml>) -> Result<Vec<String>, String> {
    match value {
        Some(Yaml::String(s)) => Ok(vec![s.clone()]),
        Some(Yaml::Array(items)) => items
            .iter()
            .map(|item| match item {
                Yaml::String(s) => Ok(s.clone()),
                other => Err(format!("unsupported 'extends' entry: {other:?}")),
            })
            .collect(),
        other => Err(format!("unsupported 'extends' value: {other:?}")),
    }
}

/// Resolve one `extends` entry: `Packages/…` is relative to the clone root,
/// anything else is relative to the extending file (Sublime accepts both).
fn resolve_parent(spec: &str, from: &Path, source_root: &Path) -> Result<PathBuf, String> {
    let candidate = match spec.strip_prefix("Packages/") {
        Some(rest) => source_root.join(rest),
        None => from
            .parent()
            .unwrap_or(source_root)
            .join(spec)
            .to_path_buf(),
    };
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(format!(
            "{}: 'extends: {spec}' not found at {}",
            from.display(),
            candidate.display()
        ))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MergeMode {
    Replace,
    Prepend,
    Append,
}

/// Split a context's patterns into the `meta_prepend`/`meta_append`
/// directive (if any) and the actual patterns. The directive is a merge
/// instruction, not a runtime rule — syntect must never see it.
fn split_merge_directives(value: &Yaml) -> (MergeMode, Vec<Yaml>) {
    let mut mode = MergeMode::Replace;
    let mut patterns = Vec::new();
    for pattern in patterns_of(value) {
        if let Yaml::Hash(map) = &pattern {
            if map.contains_key(&key("meta_prepend")) {
                mode = MergeMode::Prepend;
                continue;
            }
            if map.contains_key(&key("meta_append")) {
                mode = MergeMode::Append;
                continue;
            }
        }
        patterns.push(pattern);
    }
    (mode, patterns)
}

fn patterns_of(value: &Yaml) -> Vec<Yaml> {
    match value {
        Yaml::Array(items) => items.clone(),
        other => vec![other.clone()],
    }
}

fn emit(doc: &Yaml, path: &Path) -> Result<String, String> {
    let mut out = String::new();
    let mut emitter = YamlEmitter::new(&mut out);
    emitter
        .dump(doc)
        .map_err(|e| format!("{}: emitting flattened YAML: {e}", path.display()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests do not care about memoisation; give each call a fresh cache.
    fn flat(path: &Path, root: &Path) -> Result<String, String> {
        flatten(path, root, &mut HashMap::new())
    }

    #[test]
    fn no_inheritance_returns_the_file_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Plain.sublime-syntax");
        let source = "%YAML 1.2\n---\nname: Plain\nscope: text.plain\ncontexts:\n  main: []\n";
        std::fs::write(&path, source).unwrap();
        assert_eq!(flat(&path, dir.path()).unwrap(), source);
    }

    #[test]
    fn child_contexts_override_the_parents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Base.sublime-syntax"),
            "name: Base\nscope: source.base\ncontexts:\n  main:\n    - match: a\n      scope: keyword\n",
        )
        .unwrap();
        let child = dir.path().join("Child.sublime-syntax");
        std::fs::write(
            &child,
            "name: Child\nscope: source.child\nextends: Base.sublime-syntax\ncontexts:\n  main:\n    - match: b\n      scope: string\n",
        )
        .unwrap();
        let flat = flat(&child, dir.path()).unwrap();
        assert!(
            !flat.contains("extends"),
            "extends must be resolved: {flat}"
        );
        assert!(flat.contains("match: b"), "{flat}");
        assert!(!flat.contains("match: a"), "{flat}");
        assert!(!flat.contains("source.base"), "{flat}");
        // `scope` is not inherited.
        assert!(flat.contains("scope: source.child"), "{flat}");
    }

    #[test]
    fn meta_prepend_splices_before_the_parent_patterns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Base.sublime-syntax"),
            "name: Base\nscope: source.base\ncontexts:\n  main:\n    - match: parent-rule\n      scope: keyword\n",
        )
        .unwrap();
        let child = dir.path().join("Child.sublime-syntax");
        std::fs::write(
            &child,
            "name: Child\nscope: source.child\nextends: Base.sublime-syntax\ncontexts:\n  main:\n    - meta_prepend: true\n    - match: child-rule\n      scope: string\n",
        )
        .unwrap();
        let flat = flat(&child, dir.path()).unwrap();
        assert!(
            !flat.contains("meta_prepend"),
            "directive must be consumed: {flat}"
        );
        let child_at = flat.find("child-rule").unwrap();
        let parent_at = flat.find("parent-rule").unwrap();
        assert!(child_at < parent_at, "prepend order wrong: {flat}");
    }

    #[test]
    fn meta_append_splices_after_the_parent_patterns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Base.sublime-syntax"),
            "name: Base\nscope: source.base\ncontexts:\n  main:\n    - match: parent-rule\n      scope: keyword\n",
        )
        .unwrap();
        let child = dir.path().join("Child.sublime-syntax");
        std::fs::write(
            &child,
            "name: Child\nscope: source.child\nextends: Base.sublime-syntax\ncontexts:\n  main:\n    - meta_append: true\n    - match: child-rule\n      scope: string\n",
        )
        .unwrap();
        let flat = flat(&child, dir.path()).unwrap();
        assert!(!flat.contains("meta_append"), "{flat}");
        let child_at = flat.find("child-rule").unwrap();
        let parent_at = flat.find("parent-rule").unwrap();
        assert!(parent_at < child_at, "append order wrong: {flat}");
    }

    #[test]
    fn missing_parent_is_an_error_naming_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("Child.sublime-syntax");
        std::fs::write(
            &child,
            "name: C\nscope: source.c\nextends: Nope.sublime-syntax\ncontexts:\n  main: []\n",
        )
        .unwrap();
        let err = flat(&child, dir.path()).unwrap_err();
        assert!(err.contains("Child.sublime-syntax"), "{err}");
        assert!(err.contains("Nope.sublime-syntax"), "{err}");
    }

    #[test]
    fn cycles_are_detected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("A.sublime-syntax"),
            "name: A\nscope: source.a\nextends: B.sublime-syntax\ncontexts:\n  main: []\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("B.sublime-syntax"),
            "name: B\nscope: source.b\nextends: A.sublime-syntax\ncontexts:\n  main: []\n",
        )
        .unwrap();
        let err = flat(&dir.path().join("A.sublime-syntax"), dir.path()).unwrap_err();
        assert!(err.contains("cycle"), "{err}");
    }

    #[test]
    fn variables_merge_with_the_child_winning() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Base.sublime-syntax"),
            "name: Base\nscope: source.base\nvariables:\n  shared: from-parent\n  only_parent: yes\ncontexts:\n  main: []\n",
        )
        .unwrap();
        let child = dir.path().join("Child.sublime-syntax");
        std::fs::write(
            &child,
            "name: Child\nscope: source.child\nextends: Base.sublime-syntax\nvariables:\n  shared: from-child\ncontexts:\n  main: []\n",
        )
        .unwrap();
        let flat = flat(&child, dir.path()).unwrap();
        assert!(flat.contains("from-child"), "{flat}");
        assert!(!flat.contains("from-parent"), "{flat}");
        assert!(flat.contains("only_parent"), "{flat}");
    }
}
