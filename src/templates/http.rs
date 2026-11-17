//! Messages for a registry that refused a request.

const HINT: &str =
    "Try again later, or install a template from a local folder with 'texforge template add <folder>'.";

/// Build the user-facing message for a non-success registry response.
///
/// Only 403 and 429 are treated as a refusal that is probably a rate limit;
/// every other status returns `None` so the caller keeps its generic
/// `Registry returned HTTP <status>` message.
pub(super) fn refusal_message(
    status: u16,
    retry_after: Option<&str>,
    reset: Option<&str>,
) -> Option<String> {
    if status != 403 && status != 429 {
        return None;
    }
    let mut message = format!(
        "The template registry refused the request (HTTP {status}); this is probably a rate limit."
    );
    if let Some(value) = retry_after {
        message.push_str(&format!("\n  Retry-After: {value}"));
    }
    if let Some(value) = reset {
        message.push_str(&format!("\n  x-ratelimit-reset: {value} (Unix time)"));
    }
    message.push('\n');
    message.push_str(HINT);
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_with_both_headers_lists_them() {
        let message = refusal_message(403, Some("60"), Some("1700000000")).unwrap();
        assert!(message.contains("refused"));
        assert!(message.contains("rate limit"));
        assert!(message.contains("Retry-After: 60"));
        assert!(message.contains("x-ratelimit-reset: 1700000000 (Unix time)"));
        assert!(message.ends_with(HINT));
    }

    #[test]
    fn forbidden_without_headers_still_explains_the_refusal() {
        let message = refusal_message(403, None, None).unwrap();
        assert!(message.contains("refused"));
        assert!(!message.contains("Retry-After:"));
        assert!(!message.contains("x-ratelimit-reset:"));
        assert!(message.ends_with(HINT));
    }

    #[test]
    fn too_many_requests_is_a_refusal() {
        let message = refusal_message(429, Some("30"), None).unwrap();
        assert!(message.contains("Retry-After: 30"));
    }

    #[test]
    fn other_statuses_have_no_refusal_message() {
        assert!(refusal_message(500, Some("1"), Some("2")).is_none());
        assert!(refusal_message(404, None, None).is_none());
    }
}
