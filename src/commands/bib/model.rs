//! `.bib` parsing, identifier normalization, and title/year comparison.

use std::collections::HashSet;

/// Minimal bibliographic record extracted from a `.bib` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibRecord {
    pub key: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<String>,
    pub doi: Option<String>,
    pub eprint: Option<String>,
    pub archive_prefix: Option<String>,
    pub url: Option<String>,
}

/// Extract bibliographic records conservatively from a `.bib` source string.
///
/// Skips `@string`, `@preamble`, `@comment`, and any text/comments outside `@`
/// entries. Entries that are malformed or lack a citation key are skipped.
pub fn parse_bib_entries(source: &str) -> Vec<BibRecord> {
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut records = Vec::new();

    while i < n {
        if chars[i] == '%' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if chars[i] != '@' {
            i += 1;
            continue;
        }
        i += 1;

        let kind_start = i;
        while i < n && chars[i].is_alphanumeric() {
            i += 1;
        }
        let kind: String = chars[kind_start..i].iter().collect();
        if kind.is_empty() {
            continue;
        }

        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        let open = match chars[i] {
            '{' => '{',
            '(' => '(',
            _ => continue,
        };
        i += 1;

        let body_start = i;
        let Some(body_end) = scan_entry_body(&chars, i, open) else {
            break;
        };
        let body: String = chars[body_start..body_end].iter().collect();
        i = body_end + 1;

        let kind_l = kind.to_lowercase();
        if matches!(kind_l.as_str(), "string" | "preamble" | "comment") {
            continue;
        }
        if let Some(record) = parse_record_body(&body) {
            records.push(record);
        }
    }

    records
}

fn scan_entry_body(chars: &[char], mut i: usize, open: char) -> Option<usize> {
    let n = chars.len();
    let mut brace_depth = 0i32;
    let mut in_quote = false;
    while i < n {
        let c = chars[i];
        if in_quote {
            if c == '\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if c == '"' {
                in_quote = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' if brace_depth == 0 => in_quote = true,
            '{' => brace_depth += 1,
            '}' if open == '{' && brace_depth == 0 => return Some(i),
            '}' => brace_depth -= 1,
            ')' if open == '(' && brace_depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_record_body(body: &str) -> Option<BibRecord> {
    let segments = split_top_level(body);
    let mut iter = segments.into_iter();
    let key = iter.next()?.trim().to_string();
    if key.is_empty() || key.contains('=') {
        return None;
    }

    let mut title = None;
    let mut author = None;
    let mut year = None;
    let mut doi = None;
    let mut eprint = None;
    let mut archive_prefix = None;
    let mut url = None;
    let mut arxiv_id_field = None;

    for seg in iter {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        let Some(eq) = top_level_eq(seg) else {
            continue;
        };
        let name = seg[..eq].trim().to_lowercase();
        let val = unbrace_value(seg[eq + 1..].trim());
        if val.is_empty() {
            continue;
        }
        match name.as_str() {
            "title" => title = Some(val),
            "author" => author = Some(val),
            "year" => year = Some(val),
            "doi" => doi = Some(val),
            "eprint" => eprint = Some(val),
            "archiveprefix" => archive_prefix = Some(val),
            "arxivid" => arxiv_id_field = Some(val),
            "url" => url = Some(val),
            _ => {}
        }
    }

    if eprint.is_none() {
        if let Some(id) = arxiv_id_field {
            eprint = Some(id);
            if archive_prefix.is_none() {
                archive_prefix = Some("arXiv".to_string());
            }
        }
    }

    Some(BibRecord {
        key,
        title,
        author,
        year,
        doi,
        eprint,
        archive_prefix,
        url,
    })
}

fn split_top_level(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut prev_escape = false;

    for c in s.chars() {
        if in_quote {
            cur.push(c);
            if prev_escape {
                prev_escape = false;
            } else if c == '\\' {
                prev_escape = true;
            } else if c == '"' {
                in_quote = false;
            }
            continue;
        }
        match c {
            '"' if depth == 0 => {
                in_quote = true;
                cur.push(c);
            }
            '{' => {
                depth += 1;
                cur.push(c);
            }
            '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn top_level_eq(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_quote = false;
    for (idx, c) in s.char_indices() {
        if in_quote {
            if c == '"' {
                in_quote = false;
            }
            continue;
        }
        match c {
            '"' if depth == 0 => in_quote = true,
            '{' => depth += 1,
            '}' => depth -= 1,
            '=' if depth == 0 => return Some(idx),
            _ => {}
        }
    }
    None
}

fn unbrace_value(raw: &str) -> String {
    let mut s = raw.trim();
    loop {
        let is_wrapped = (s.starts_with('{') && s.ends_with('}') && wraps_whole_braced(s))
            || (s.starts_with('"') && s.ends_with('"') && s.len() >= 2 && wraps_whole_quoted(s));
        if is_wrapped {
            s = s[1..s.len() - 1].trim();
        } else {
            break;
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn wraps_whole_braced(s: &str) -> bool {
    let mut depth = 0i32;
    for (idx, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 && idx < s.len() - 1 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn wraps_whole_quoted(s: &str) -> bool {
    let mut prev_escape = false;
    for (idx, c) in s.char_indices().skip(1) {
        if prev_escape {
            prev_escape = false;
            continue;
        }
        if c == '\\' {
            prev_escape = true;
            continue;
        }
        if c == '"' {
            return idx == s.len() - 1;
        }
    }
    false
}

/// Normalize a DOI string from `10.xxx/...`, `https://doi.org/...`,
/// `http://dx.doi.org/...`, or `doi:...` forms into `10.xxx/...`.
pub fn normalize_doi(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    let lower = s.to_ascii_lowercase();
    for prefix in [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "doi:",
    ] {
        if lower.starts_with(prefix) {
            s = &s[prefix.len()..];
            break;
        }
    }
    let normalized = s.trim().to_ascii_lowercase();
    let rest = normalized.strip_prefix("10.")?;
    let (prefix, suffix) = rest.split_once('/')?;
    if prefix.is_empty() || suffix.is_empty() {
        return None;
    }
    Some(normalized)
}

/// Return `true` when a normalized DOI is an arXiv `DataCite` DOI (`10.48550/arXiv.<id>`).
pub fn is_arxiv_doi(normalized_doi: &str) -> bool {
    normalized_doi.starts_with("10.48550/arxiv.")
}

/// Extract an arXiv identifier from `doi`, `eprint` + `archivePrefix = arXiv`,
/// or an `arxiv.org/abs/<id>` (or `/pdf/<id>`) URL.
pub fn extract_arxiv_id(record: &BibRecord) -> Option<String> {
    if let Some(doi_raw) = &record.doi {
        if let Some(doi) = normalize_doi(doi_raw) {
            if let Some(id) = doi.strip_prefix("10.48550/arxiv.") {
                let id = id.trim();
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }

    if let (Some(eprint), Some(prefix)) = (&record.eprint, &record.archive_prefix) {
        if prefix.trim().eq_ignore_ascii_case("arxiv") {
            let trimmed = eprint.trim();
            let id = if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("arxiv:") {
                trimmed[6..].trim()
            } else {
                trimmed
            };
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }

    if let Some(url) = &record.url {
        let lower = url.to_ascii_lowercase();
        for marker in ["arxiv.org/abs/", "arxiv.org/pdf/"] {
            if let Some(pos) = lower.find(marker) {
                let after = &url[pos + marker.len()..];
                let end = after.find(['?', '#']).unwrap_or(after.len());
                let mut id = after[..end].trim().trim_end_matches('/');
                if let Some(stripped) = id.strip_suffix(".pdf") {
                    id = stripped;
                }
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }

    None
}

/// Normalize a title for comparison: strip LaTeX commands and braces, fold
/// diacritics and punctuation, lowercase, and collapse whitespace.
pub fn normalize_title(raw: &str) -> String {
    let stripped = strip_latex(raw);
    let mut folded = String::with_capacity(stripped.len());
    for c in stripped.chars() {
        if ('\u{0300}'..='\u{036F}').contains(&c) {
            continue;
        }
        let base = fold_diacritic(c);
        for lc in base.to_lowercase() {
            if lc.is_alphanumeric() {
                folded.push(lc);
            } else {
                folded.push(' ');
            }
        }
    }
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_latex(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < n {
        match chars[i] {
            '{' | '}' => i += 1,
            '\\' => {
                i += 1;
                if i >= n {
                    break;
                }
                if chars[i].is_ascii_alphabetic() {
                    while i < n && chars[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    if i < n && chars[i] == '*' {
                        i += 1;
                    }
                } else {
                    if matches!(chars[i], '\\' | '-' | ' ') {
                        out.push(' ');
                    }
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

fn fold_diacritic(c: char) -> char {
    match c {
        'À'..='Å' | 'à'..='å' => 'a',
        'Ç' | 'ç' | 'Ć'..='Č' | 'ć'..='č' => 'c',
        'Ď' | 'ď' | 'Đ' | 'đ' => 'd',
        'È'..='Ë' | 'è'..='ë' | 'Ē'..='Ě' | 'ē'..='ě' => 'e',
        'Ĝ'..='Ģ' | 'ĝ'..='ģ' => 'g',
        'Ĥ'..='Ħ' | 'ĥ'..='ħ' => 'h',
        'Ì'..='Ï' | 'ì'..='ï' | 'Ĩ'..='İ' | 'ĩ'..='ı' => 'i',
        'Ĵ' | 'ĵ' => 'j',
        'Ķ' | 'ķ' => 'k',
        'Ĺ'..='Ł' | 'ĺ'..='ł' => 'l',
        'Ñ' | 'ñ' | 'Ń'..='Ň' | 'ń'..='ň' => 'n',
        'Ò'..='Ö' | 'Ø' | 'ò'..='ö' | 'ø' | 'Ō'..='Ő' | 'ō'..='ő' => 'o',
        'Ŕ'..='Ř' | 'ŕ'..='ř' => 'r',
        'Ś'..='Š' | 'ś'..='š' => 's',
        'Ţ'..='Ŧ' | 'ţ'..='ŧ' => 't',
        'Ù'..='Ü' | 'ù'..='ü' | 'Ũ'..='Ű' | 'ũ'..='ű' => 'u',
        'Ŵ' | 'ŵ' => 'w',
        'Ý' | 'Ÿ' | 'ý' | 'ÿ' | 'Ŷ' | 'ŷ' => 'y',
        'Ź'..='Ž' | 'ź'..='ž' => 'z',
        other => other,
    }
}

/// Compare two titles after normalization: match if equal, if one contains the
/// other, or if their token-level Jaccard similarity is at least `0.6`.
pub fn title_matches(expected: &str, found: &str) -> bool {
    let a = normalize_title(expected);
    let b = normalize_title(found);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b || a.contains(&b) || b.contains(&a) {
        return true;
    }
    let ta: HashSet<&str> = a.split_whitespace().collect();
    let tb: HashSet<&str> = b.split_whitespace().collect();
    let intersection = ta.intersection(&tb).count();
    let union = ta.union(&tb).count();
    union > 0 && (intersection as f64 / union as f64) >= 0.6
}

/// Return `Some((bib_year, issued_year))` if both years are present and differ by
/// more than 1 year.
pub fn year_drift(bib_year: Option<&str>, issued_year: Option<i32>) -> Option<(i32, i32)> {
    let issued = issued_year?;
    let raw = bib_year?;
    let bib = parse_four_digit_year(raw)?;
    if (bib - issued).abs() > 1 {
        Some((bib, issued))
    } else {
        None
    }
}

fn parse_four_digit_year(raw: &str) -> Option<i32> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 4 {
        digits[..4].parse::<i32>().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doi_forms_normalize_consistently() {
        assert_eq!(
            normalize_doi("10.1145/3290605.3300233").as_deref(),
            Some("10.1145/3290605.3300233")
        );
        assert_eq!(
            normalize_doi("https://doi.org/10.1145/3290605.3300233").as_deref(),
            Some("10.1145/3290605.3300233")
        );
        assert_eq!(
            normalize_doi("http://dx.doi.org/10.1145/3290605.3300233").as_deref(),
            Some("10.1145/3290605.3300233")
        );
        assert_eq!(
            normalize_doi("doi: 10.1145/3290605.3300233").as_deref(),
            Some("10.1145/3290605.3300233")
        );
        assert_eq!(
            normalize_doi("DOI:10.1000/XYZ").as_deref(),
            Some("10.1000/xyz")
        );
        assert_eq!(normalize_doi("not-a-doi"), None);
    }

    #[test]
    fn arxiv_forms_extract_from_eprint_url_and_doi() {
        let from_eprint = BibRecord {
            key: "k1".into(),
            title: None,
            author: None,
            year: None,
            doi: None,
            eprint: Some("2301.00001".into()),
            archive_prefix: Some("arXiv".into()),
            url: None,
        };
        assert_eq!(
            extract_arxiv_id(&from_eprint).as_deref(),
            Some("2301.00001")
        );

        let from_url = BibRecord {
            key: "k2".into(),
            title: None,
            author: None,
            year: None,
            doi: None,
            eprint: None,
            archive_prefix: None,
            url: Some("https://arxiv.org/abs/2103.14030v2?foo=1".into()),
        };
        assert_eq!(extract_arxiv_id(&from_url).as_deref(), Some("2103.14030v2"));

        let from_doi = BibRecord {
            key: "k3".into(),
            title: None,
            author: None,
            year: None,
            doi: Some("10.48550/arXiv.1706.03762".into()),
            eprint: None,
            archive_prefix: None,
            url: None,
        };
        assert_eq!(extract_arxiv_id(&from_doi).as_deref(), Some("1706.03762"));
    }

    #[test]
    fn title_match_equal_contains_and_jaccard() {
        // Equal after LaTeX + diacritic folding
        assert!(title_matches(
            r"R{\'{e}}sum{\'{e}} of \emph{Deep} Learning",
            "Résumé of Deep Learning"
        ));
        // Containment (subtitle omitted)
        assert!(title_matches(
            "Attention Is All You Need",
            "Attention Is All You Need: A Survey"
        ));
        // Jaccard >= 0.6 (4 / 5 = 0.8)
        assert!(title_matches(
            "Fast Neural Rendering of Scenes",
            "Fast Neural Rendering for Scenes"
        ));
        // Different work
        assert!(!title_matches(
            "Quantum Computing in Practice",
            "Classical Mechanics for Engineers"
        ));
    }

    #[test]
    fn year_drift_warns_only_above_one_year() {
        assert_eq!(year_drift(Some("2022"), Some(2023)), None);
        assert_eq!(year_drift(Some("2023"), Some(2022)), None);
        assert_eq!(year_drift(Some("2020"), Some(2023)), Some((2020, 2023)));
        assert_eq!(year_drift(None, Some(2023)), None);
    }

    #[test]
    fn parse_bib_entries_extracts_fields_and_skips_comments() {
        let bib = r#"
% Leading comment
@string{acl = "ACL"}
@article{vaswani2017,
  author = {Vaswani, Ashish and others},
  title = {{Attention Is All You Need}},
  year = 2017,
  doi = {https://doi.org/10.48550/arXiv.1706.03762},
}
@misc{other,
  title = "Another Paper",
  eprint = {2301.00001},
  archivePrefix = {arXiv},
}
"#;
        let entries = parse_bib_entries(bib);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "vaswani2017");
        assert_eq!(
            entries[0].title.as_deref(),
            Some("Attention Is All You Need")
        );
        assert_eq!(entries[0].year.as_deref(), Some("2017"));
        assert_eq!(entries[1].key, "other");
        assert_eq!(entries[1].eprint.as_deref(), Some("2301.00001"));
        assert_eq!(entries[1].archive_prefix.as_deref(), Some("arXiv"));
    }
}
