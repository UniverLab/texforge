//! Path checks for entries coming out of a downloaded archive.

/// True when `rel` names a file inside the template directory and nothing
/// outside it: not empty, not absolute, no `..` component, no backslash and
/// no Windows drive prefix.
pub(crate) fn is_safe_relative(rel: &str) -> bool {
    if rel.is_empty() || rel.starts_with('/') || rel.contains('\\') {
        return false;
    }
    let bytes = rel.as_bytes();
    if bytes[0].is_ascii_alphabetic() && bytes.get(1) == Some(&b':') {
        return false;
    }
    !rel.split('/').any(|component| component == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_nested_relative_path() {
        assert!(is_safe_relative("a/b.tex"));
        assert!(is_safe_relative("main.tex"));
    }

    #[test]
    fn rejects_dotdot_and_absolute_paths() {
        assert!(!is_safe_relative("../x"));
        assert!(!is_safe_relative("/x"));
        assert!(!is_safe_relative("a/../../x"));
        assert!(!is_safe_relative("a/.."));
    }

    #[test]
    fn rejects_empty_and_windows_forms() {
        assert!(!is_safe_relative(""));
        assert!(!is_safe_relative("..\\x"));
        assert!(!is_safe_relative("C:/x"));
    }
}
