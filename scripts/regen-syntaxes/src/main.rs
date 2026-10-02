//! Regenerate `assets/highlight/syntaxes.syntect` from a `sublimehq/Packages` clone.
//!
//! The dump holds only the curated language manifest below — not syntect's
//! full default set — so the shipped binary stays inside its size budget and
//! never embeds yaml/plist parsing. Network happens ONLY here, run by hand by
//! a maintainer; `cargo build` of texforge itself and the runtime never touch
//! the network.
//!
//! Selection is *reference-closure*, not "every file in the folder": the
//! manifest folders are walked for their top-level syntaxes, then whatever
//! those reference by file or by scope (`push`/`set`/`include`/`embed` targets
//! such as `Packages/PHP/Embeddings/…` or `scope:source.go.embedded-…`) is
//! pulled in transitively — syntect resolves those at parse time, and an
//! unresolvable non-`embed` reference is a parse error (which the highlight
//! pass would have to degrade to monochrome). Everything *not* referenced
//! stays out: upstream folders carry dozens of near-duplicate embedding
//! grammars that would otherwise double the dump.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use syntect::dumps::dump_binary;
use syntect::parsing::{SyntaxDefinition, SyntaxSetBuilder};
use yaml_rust::{Yaml, YamlLoader};

mod inherit;

/// Folders of `sublimehq/Packages` compiled into the shipped dump.
///
/// `C++` covers both C and C++ (`C.sublime-syntax` lives there upstream);
/// `JavaScript` covers JavaScript, JSX, TypeScript and TSX. `C#` and
/// `Dockerfile` are deliberately absent: no folder exists upstream for
/// Dockerfile, and C# is excluded by the size-budget manifest.
const MANIFEST: &[&str] = &[
    "ShellScript",
    "C++",
    "Clojure",
    "CSS",
    "Diff",
    "Erlang",
    "Git Formats",
    "Go",
    "Graphviz",
    "HTML",
    "Haskell",
    "JSON",
    "Java",
    "JavaScript",
    "LaTeX",
    "Lua",
    "Makefile",
    "Markdown",
    "Matlab",
    "OCaml",
    "PHP",
    "Python",
    "R",
    "Ruby",
    "Rust",
    "SQL",
    "TOML",
    "XML",
    "YAML",
];

/// Sublime hands lines to its regex engine *with* trailing newlines, which is
/// how syntect recommends loading syntaxes; the highlight pass feeds
/// `highlight_line` exactly that shape.
const LINES_INCLUDE_NEWLINE: bool = true;

/// Reference kinds a syntax can use to pull in another syntax. `embed`/`escape`
/// fall back to plain text when the target is missing; the rest do not, so an
/// unresolved one is reported below instead of silently shipping.
const REF_KEYS: &[&str] = &["push", "set", "include", "embed", "escape"];

/// Pattern rewrites applied to every flattened syntax, so that grammars
/// written for Sublime's `onig` engine still work on the pure-Rust one texforge
/// links (`fancy-regex`; a C toolchain is out of the question). Each entry must
/// be *match-equivalent*, never a semantic downgrade, and every hit is printed
/// so a regen never changes behaviour silently.
///
/// * `\<+` — a repeated word-boundary assertion. `fancy-regex` rejects
///   repeating a zero-width assertion ("target of repeat operator is
///   invalid") while `onig` accepts it. A repeated `\<` can only ever match
///   where a single `\<` does (the boundary is already consumed), so the
///   single assertion is the same match set. Upstream uses it in Zsh's
///   `*-glob-range-fallback` contexts, whose job is to scope a stray `<`
///   literally.
/// * `\>+` — symmetric to `\<+`, same reasoning for end-of-word boundary.
/// * `(?:\<)+` / `(?:\>)+` — non-capturing groups repeating a zero-width
///   assertion; same issue, same fix.
/// * `\b+` / `(?:\b)+` — word boundary repeated; `fancy-regex` rejects this.
///   A single `\b` matches the same positions.
///
/// The rewrite happens on the **decoded** pattern, after YAML parsing: the
/// emitter re-escapes backslashes, so patching the raw file text would miss
/// exactly the grammars that need it most.
const PATCHES: &[(&str, &str)] = &[
    ("\\<+", "\\<"),
    ("\\>+", "\\>"),
    ("(?:\\<)+", "\\<"),
    ("(?:\\>)+", "\\>"),
    ("\\b+", "\\b"),
    ("(?:\\b)+", "\\b"),
    // Upstream's fenced-code closing pattern repeats backreferences:
    // `(?:(?:\2)*|(?:\3)*)` — "as many more delimiter chars as the opener
    // had". fancy-regex rejects a repeated backreference, and syntect's own
    // line-time substitution makes it worse: `\3` is absent for backtick
    // fences (the tilde branch of the opener never ran), the substituter
    // drops it, and the pattern becomes `(?:)*` — an empty repeated group,
    // which fancy-regex also rejects (panic inside `highlight_line`). Both
    // delimiter kinds are backticks or tildes, so the literal class matches
    // the same closing fences *and* survives every substitution.
    ("(?:(?:\\2)*|(?:\\3)*)", "[`~]*"),
];

/// Apply [`PATCHES`] and [`relax_repeated_backrefs`] to one loaded syntax,
/// reporting every hit: a silent grammar edit is exactly the kind of thing a
/// regenerate must never do unnoticed. Returns the number of rewritten
/// patterns.
fn patch_definition(def: &mut SyntaxDefinition, label: &str) -> usize {
    use syntect::parsing::syntax_definition::{MatchPattern, Pattern};

    let mut hits = 0usize;
    for context in def.contexts.values_mut() {
        for pattern in context.patterns.iter_mut() {
            let Pattern::Match(matcher) = pattern else {
                continue;
            };
            let stored = matcher.regex.regex_str().to_string();
            let mut patched = stored.clone();
            for (from, to) in PATCHES {
                patched = patched.replace(from, to);
            }
            patched = relax_repeated_backrefs(&patched);
            if patched == stored {
                continue;
            }
            *pattern = Pattern::Match(MatchPattern::new(
                matcher.has_captures,
                patched.clone(),
                matcher.scope.clone(),
                matcher.captures.clone(),
                matcher.operation.clone(),
                matcher.with_prototype.clone(),
            ));
            hits += 1;
            println!("patched: {label}: {stored:?} → {patched:?}");
        }
    }
    hits
}

/// `(?:\2)*` → `(?:\2)?`: a *repeated backreference group*.
///
/// `fancy-regex` cannot compile a repetition whose target is a backreference
/// ("target of repeat operator is invalid"), and — unlike syntect's intended
/// path, where such a pattern is recompiled per line with the captured text
/// substituted in — a pattern that also sits in a prototype context is
/// compiled verbatim, which panics inside `highlight_line` and takes the
/// user's build with it. Upstream's only use is Markdown's
/// `fenced_code_block_end`, which accepts a closing fence one *extra* run
/// longer than the opening one; `?` keeps every real-world fence and gives up
/// only the pathological ``` ``` ``` ``` ``` case.
///
/// Cheap, local, and reported on every regen.
fn relax_repeated_backrefs(regex: &str) -> String {
    let chars: Vec<char> = regex.chars().collect();
    let mut out = String::with_capacity(regex.len());
    let mut i = 0usize;
    while i < chars.len() {
        // A group whose whole body is a backreference: `(?:\1)` or `(\1)`.
        let prefix = match chars.get(i) {
            Some('(') if chars.get(i + 1) == Some(&'?') && chars.get(i + 2) == Some(&':') => 4,
            Some('(') => 2,
            _ => 0,
        };
        if prefix > 0 && chars.get(i + prefix - 1) == Some(&'\\') {
            let mut end = i + prefix;
            while chars.get(end).is_some_and(char::is_ascii_digit) {
                end += 1;
            }
            let group_closed = end > i + prefix && chars.get(end) == Some(&')');
            let after = if group_closed { end + 1 } else { end };
            if group_closed && matches!(chars.get(after), Some('*') | Some('+')) {
                out.extend(chars[i..after].iter());
                out.push('?');
                i = after + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn usage() -> ! {
    eprintln!("usage: regen-syntaxes [--source <path-to-Packages-clone>] [--out <path>]");
    std::process::exit(2);
}

fn main() {
    let mut source = PathBuf::from("./vendor/Packages");
    let mut out = PathBuf::from("../../assets/highlight/syntaxes.syntect");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source" => source = args.next().unwrap_or_else(|| usage()).into(),
            "--out" => out = args.next().unwrap_or_else(|| usage()).into(),
            _ => usage(),
        }
    }

    if !source.is_dir() {
        eprintln!(
            "error: {} is not a directory — clone it first:\n  git clone --depth 1 \
             https://github.com/sublimehq/Packages {}",
            source.display(),
            source.display()
        );
        std::process::exit(1);
    }
    // Canonicalize before anything is keyed by path: the flatten cache stores
    // canonical paths, so a relative `--source` would make every later lookup
    // miss and panic on `cache[file]`.
    let source = source.canonicalize().unwrap_or(source);

    // Every manifest file, flattened out of Sublime's `extends:` inheritance
    // (syntect 5.3 cannot read that key at all) and memoised, since parents
    // are shared by many children. A file that still fails aborts the run
    // loudly — a silently thinner dump must be impossible to ship.
    let mut cache: HashMap<PathBuf, String> = HashMap::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    for folder in MANIFEST {
        let dir = source.join(folder);
        if !dir.is_dir() {
            eprintln!(
                "error: manifest folder '{folder}' missing under {} — \
                 update MANIFEST in scripts/regen-syntaxes/src/main.rs, never ship a partial dump",
                source.display()
            );
            std::process::exit(1);
        }
        roots.extend(syntax_files(&dir).into_iter().filter(|p| {
            // Top-level of the folder only; `Embeddings/` enters through the
            // closure below, if and only if something references it.
            p.parent().map(Path::to_path_buf) == Some(dir.clone())
        }));
    }
    roots.sort();
    for file in &roots {
        if let Err(e) = inherit::flatten(file, &source, &mut cache) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }

    let (by_name, by_scope) = index_clone(&source);
    let selected = closure(&roots, &mut cache, &by_name, &by_scope, &source);

    // Every pattern in the dump must compile under the SAME engine the shipped
    // binary uses (fancy-regex, no onig), after the documented rewrites. A
    // pattern that only breaks at tokenize time would otherwise panic inside a
    // user's `texforge build`. Verify before writing.
    let mut incompatible = Vec::new();

    let mut builder = SyntaxSetBuilder::new();
    let mut payload = 0usize;
    let mut loaded: Vec<(String, SyntaxDefinition)> = Vec::new();
    for file in &selected {
        let rel = file
            .strip_prefix(&source)
            .unwrap_or(file)
            .display()
            .to_string();
        let text = &cache[file];
        let mut def = match inherit::load_from_flattened(text, file, LINES_INCLUDE_NEWLINE) {
            Ok(def) => def,
            Err(e) => {
                eprintln!("error: loading {rel} failed: {e}");
                std::process::exit(1);
            }
        };
        patch_definition(&mut def, &rel);
        for offender in engine_incompatible(&def) {
            incompatible.push(format!("{rel}: {offender}"));
        }
        let size = dump_binary(&def).len();
        payload += size;
        println!("{size:>8}B  {rel}  ({})", def.name);
        loaded.push((rel, def));
    }
    println!(
        "{:>8}B  per-syntax payloads before linking ({} syntaxes)",
        payload,
        loaded.len()
    );
    if !incompatible.is_empty() {
        eprintln!(
            "error: {} pattern(s) do not compile under fancy-regex, which is the \
             only regex engine texforge links (no C toolchain, no onig):",
            incompatible.len()
        );
        for offender in &incompatible {
            eprintln!("  {offender}");
        }
        eprintln!(
            "hint: drop the providing language from MANIFEST, or add a PATCHES entry \
             rewriting the pattern to an equivalent fancy-regex-compatible form"
        );
        std::process::exit(1);
    }

    // Plain-text fallback so `lang=text` resolves to a real (rule-less) syntax.
    builder.add_plain_text_syntax();
    for (_, def) in loaded {
        builder.add(def);
    }

    let set = builder.build();
    let bytes = dump_binary(&set);
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).unwrap();
        }
    }
    std::fs::write(&out, &bytes).unwrap();
    println!(
        "wrote {} ({} syntaxes, {} bytes)",
        out.display(),
        set.syntaxes().len(),
        bytes.len()
    );
}

/// Walk every pattern of one loaded syntax and report the ones the shipped
/// binary's regex engine cannot compile. Uses `fancy-regex` directly — the
/// exact crate `syntect` links when built with `regex-fancy` — so the check
/// cannot drift from the runtime.
///
/// A pattern is tested exactly as the runtime would compile it: syntect
/// rewrites `\N` into the *escaped text* a previous pattern captured
/// (`MatchPattern::regex_with_refs`) for every context marked
/// `uses_backrefs`, and compiles the stored string verbatim everywhere else.
/// Testing the stored `\N` unconditionally would therefore report a pile of
/// false positives (Sublime's heredoc rules, Markdown's blockquote
/// continuation, …) that never reach the engine; testing the substituted form
/// catches exactly what the shipped binary breaks on.
fn engine_incompatible(def: &SyntaxDefinition) -> Vec<String> {
    use syntect::parsing::syntax_definition::Pattern;

    let mut out = Vec::new();
    if let Some(first) = &def.first_line_match {
        if let Err(e) = fancy_regex::Regex::new(first) {
            out.push(format!("{}/first_line_match: {e}: {first}", def.name));
        }
    }
    for (context_name, context) in &def.contexts {
        for pattern in &context.patterns {
            let Pattern::Match(matcher) = pattern else {
                continue;
            };
            let stored = matcher.regex.regex_str();
            let effective = if context.uses_backrefs {
                substitute_backrefs(stored)
            } else {
                stored.to_string()
            };
            if let Err(e) = fancy_regex::Regex::new(&effective) {
                out.push(format!(
                    "{}/context {context_name}: {e}: {effective}",
                    def.name
                ));
            }
            // The other runtime shape: when the referenced group did not
            // participate in the prior match, syntect *drops* the
            // backreference (its substituter returns nothing for a missing
            // group). A pattern left quantifying an empty group — `(?:)*` —
            // is rejected by fancy-regex and panics in `highlight_line`, so
            // test that shape too.
            let dropped = drop_backrefs(stored);
            if dropped != effective {
                if let Err(e) = fancy_regex::Regex::new(&dropped) {
                    out.push(format!(
                        "{}/context {context_name} (group absent): {e}: {dropped}",
                        def.name
                    ));
                }
            }
        }
    }
    out
}

/// [`substitute_backrefs`] with every backreference *removed*, mimicking
/// `syntect::parsing`'s substitution for a group that did not participate.
fn drop_backrefs(regex: &str) -> String {
    let mut out = String::with_capacity(regex.len());
    let mut chars = regex.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some(d) if d.is_ascii_digit() => {
                while chars.peek().is_some_and(|d| d.is_ascii_digit()) {
                    chars.next();
                }
            }
            _ => {
                out.push(c);
                if let Some(d) = chars.next() {
                    out.push(d);
                }
            }
        }
    }
    out
}

/// Stand in for the text a backreference expands to at parse time. A plain
/// letter is the conservative choice: it is a valid literal everywhere, so a
/// pattern that fails here fails for every possible capture too.
fn substitute_backrefs(regex: &str) -> String {
    let mut out = String::with_capacity(regex.len());
    let mut chars = regex.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some(d) if d.is_ascii_digit() => {
                while chars.peek().is_some_and(|d| d.is_ascii_digit()) {
                    chars.next();
                }
                out.push('x');
            }
            _ => {
                out.push(c);
                if let Some(d) = chars.next() {
                    out.push(d);
                }
            }
        }
    }
    out
}

/// Transitive reference closure: start from the manifest top-level syntaxes
/// and add every syntax they reference by file name or by scope, using the
/// *flattened* text so references inherited through `extends` count too.
fn closure(
    roots: &[PathBuf],
    cache: &mut HashMap<PathBuf, String>,
    by_name: &HashMap<String, PathBuf>,
    by_scope: &HashMap<String, PathBuf>,
    source: &Path,
) -> Vec<PathBuf> {
    let mut selected: BTreeSet<PathBuf> = roots.iter().cloned().collect();
    let mut queue: Vec<PathBuf> = roots.to_vec();
    let mut unresolved: Vec<String> = Vec::new();

    while let Some(file) = queue.pop() {
        let Some(text) = cache.get(&file).cloned() else {
            continue;
        };
        for (kind, target) in refs_in(&text) {
            let resolved = if let Some(stem) = target.strip_suffix(".sublime-syntax") {
                let stem = stem.rsplit('/').next().unwrap_or(stem);
                by_name.get(stem)
            } else if let Some(scope) = target.strip_prefix("scope:") {
                by_scope.get(scope)
            } else {
                None
            };
            match resolved {
                Some(path) if is_manifest_file(path, source) => {
                    if !cache.contains_key(path) {
                        // A syntax referenced from outside the top-level walk
                        // (`Embeddings/…`): flatten it before loading.
                        if let Err(e) = inherit::flatten(path, source, cache) {
                            eprintln!("error: {e}");
                            std::process::exit(1);
                        }
                    }
                    if selected.insert(path.clone()) {
                        queue.push(path.clone());
                    }
                }
                Some(path) => {
                    // Resolvable, but the providing syntax is outside the
                    // budget manifest. `embed` falls back to plain text;
                    // anything else would be a parse error — say so.
                    if kind != "embed" && kind != "escape" {
                        unresolved.push(format!(
                            "{}: {kind}: {target} (provided by {})",
                            file.display(),
                            path.display()
                        ));
                    }
                }
                None => {
                    if kind != "embed" && kind != "escape" {
                        unresolved.push(format!(
                            "{}: {kind}: {target} (not found in the clone)",
                            file.display()
                        ));
                    }
                }
            }
        }
    }

    for u in &unresolved {
        eprintln!("warning: unresolved reference — {u}");
    }

    let mut out: Vec<PathBuf> = selected.into_iter().collect();
    out.sort();
    out
}

/// Whether `path` lives in one of the manifest folders.
fn is_manifest_file(path: &Path, source: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(source) else {
        return false;
    };
    rel.components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .is_some_and(|folder| MANIFEST.contains(&folder))
        && rel.components().count() >= 2
}

/// Collect `(kind, target)` references from one flattened syntax file.
/// `extends:` is skipped on purpose: the parent's content is already merged
/// into the child, so the parent must not be pulled into the set twice.
fn refs_in(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(docs) = YamlLoader::load_from_str(text) else {
        return out;
    };
    let Some(Yaml::Hash(root)) = docs.first() else {
        return out;
    };
    for (key, value) in root {
        if key == &Yaml::String("extends".to_string()) {
            continue;
        }
        collect_refs(key, value, &mut out);
    }
    out
}

fn collect_refs(key: &Yaml, value: &Yaml, out: &mut Vec<(String, String)>) {
    let interesting = matches!(key, Yaml::String(s) if REF_KEYS.contains(&s.as_str()));
    match value {
        Yaml::String(s) => {
            if interesting {
                let target = s.split('#').next().unwrap_or(s);
                if target.ends_with(".sublime-syntax") || target.starts_with("scope:") {
                    out.push((
                        key.as_str().unwrap_or_default().to_string(),
                        target.to_string(),
                    ));
                }
            }
        }
        Yaml::Array(items) => {
            for item in items {
                collect_refs(key, item, out);
            }
        }
        Yaml::Hash(map) => {
            for (k, v) in map {
                collect_refs(k, v, out);
            }
        }
        _ => {}
    }
}

/// Clone-wide indexes so a reference can be resolved to its file: declared
/// syntax `name` (what syntect compares against) and file stem → file, plus
/// top-level `scope:` → file.
fn index_clone(source: &Path) -> (HashMap<String, PathBuf>, HashMap<String, PathBuf>) {
    let mut by_name = HashMap::new();
    let mut by_scope = HashMap::new();
    for file in sorted_syntax_files(source) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(docs) = YamlLoader::load_from_str(&text) else {
            continue;
        };
        let Some(Yaml::Hash(root)) = docs.first() else {
            continue;
        };
        if let Some(Yaml::String(name)) = root.get(&Yaml::String("name".to_string())) {
            by_name.entry(name.clone()).or_insert_with(|| file.clone());
        }
        if let Some(Yaml::String(scope)) = root.get(&Yaml::String("scope".to_string())) {
            by_scope
                .entry(scope.clone())
                .or_insert_with(|| file.clone());
        }
        if let Some(stem) = file.file_stem().and_then(|s| s.to_str()) {
            by_name
                .entry(stem.to_string())
                .or_insert_with(|| file.clone());
        }
    }
    (by_name, by_scope)
}

/// Every `.sublime-syntax` under `dir`, recursively (upstream keeps
/// cross-referencing syntaxes in `Embeddings/` subfolders).
fn syntax_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(syntax_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "sublime-syntax") {
            out.push(path);
        }
    }
    out
}

fn sorted_syntax_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = syntax_files(dir);
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_backref_groups_become_optional() {
        assert_eq!(
            relax_repeated_backrefs(r"(?:(?:\2)*|(?:\3)*)"),
            r"(?:(?:\2)?|(?:\3)?)"
        );
        assert_eq!(relax_repeated_backrefs(r"(\1)+"), r"(\1)?");
        assert_eq!(relax_repeated_backrefs(r"(\12)*"), r"(\12)?");
    }

    #[test]
    fn ordinary_repeats_and_escapes_are_left_alone() {
        for regex in [
            r"(?:ab)*",
            r"\1*",
            r"(a+)+",
            r"(?:(?:\d)*)*",
            r"\\<+",
            r"(?:(\2))*",
        ] {
            assert_eq!(relax_repeated_backrefs(regex), regex, "{regex}");
        }
    }

    #[test]
    fn the_documented_patch_rewrites_the_boundary_repeat() {
        let (from, to) = PATCHES[0];
        assert_eq!(r"    - match: \<+".replace(from, to), r"    - match: \<");
    }
}
