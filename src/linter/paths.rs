//! Path predicates shared by the file-existence checks.

/// A path template (`#1`..`#9`), not a path.
///
/// A macro parameter inside a `\newcommand` body is substituted when the macro
/// runs, so the argument the linter sees cannot be checked against the disk.
pub(super) fn has_macro_parameter(arg: &str) -> bool {
    let bytes = arg.as_bytes();
    bytes
        .iter()
        .enumerate()
        .any(|(i, byte)| *byte == b'#' && matches!(bytes.get(i + 1), Some(b'1'..=b'9')))
}
