//! Semantic versioning utilities for version comparison and stability detection.

use std::cmp::Ordering;

/// A semantic version parsed as `(major, minor, patch, prerelease_suffix)`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub prerelease: Option<String>,
}

impl SemVer {
    /// Parse a version string like "1.2.3" or "1.2.3-alpha" into a `SemVer`.
    /// Handles common prefixes like "v1.2.3".
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim_start_matches('v');

        // Split on '-' to separate prerelease
        let (version_part, prerelease) = if let Some(idx) = s.find('-') {
            let (v, p) = s.split_at(idx);
            (v, Some(p[1..].to_string())) // skip the '-'
        } else {
            (s, None)
        };

        // Parse major.minor.patch
        let parts: Vec<&str> = version_part.split('.').collect();
        if parts.len() != 3 {
            return None;
        }

        let major = parts[0].parse::<u32>().ok()?;
        let minor = parts[1].parse::<u32>().ok()?;
        let patch = parts[2].parse::<u32>().ok()?;

        Some(SemVer {
            major,
            minor,
            patch,
            prerelease,
        })
    }

    /// Check if this version is stable (no prerelease suffix).
    #[allow(dead_code)]
    pub fn is_stable(&self) -> bool {
        self.prerelease.is_none()
    }

    /// Compare two versions. Returns `Ordering::Greater` if self > other.
    /// Stable versions are considered greater than prerelease versions with same base.
    pub fn compare(&self, other: &SemVer) -> Ordering {
        // First compare base version (major.minor.patch)
        match self.major.cmp(&other.major) {
            Ordering::Equal => {}
            other_order => return other_order,
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => {}
            other_order => return other_order,
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => {}
            other_order => return other_order,
        }

        // Base version is equal. Compare prerelease.
        // Stable > prerelease for same base version
        match (&self.prerelease, &other.prerelease) {
            (None, None) => Ordering::Equal,      // both stable
            (None, Some(_)) => Ordering::Greater, // self stable, other prerelease
            (Some(_), None) => Ordering::Less,    // self prerelease, other stable
            (Some(a), Some(b)) => a.cmp(b),       // both prerelease, compare strings
        }
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &SemVer) -> Option<Ordering> {
        Some(std::cmp::Ord::cmp(self, other))
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &SemVer) -> Ordering {
        self.compare(other)
    }
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.prerelease {
            write!(f, "-{}", pre)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let v = SemVer::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert!(v.is_stable());
    }

    #[test]
    fn test_parse_with_v_prefix() {
        let v = SemVer::parse("v1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_parse_prerelease() {
        let v = SemVer::parse("1.2.3-alpha").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.prerelease, Some("alpha".to_string()));
        assert!(!v.is_stable());
    }

    #[test]
    fn test_parse_invalid() {
        assert!(SemVer::parse("1.2").is_none());
        assert!(SemVer::parse("a.b.c").is_none());
    }

    #[test]
    fn test_compare_major() {
        let v1 = SemVer::parse("2.0.0").unwrap();
        let v2 = SemVer::parse("1.9.9").unwrap();
        assert!(v1 > v2);
    }

    #[test]
    fn test_compare_minor() {
        let v1 = SemVer::parse("1.2.0").unwrap();
        let v2 = SemVer::parse("1.1.9").unwrap();
        assert!(v1 > v2);
    }

    #[test]
    fn test_compare_patch() {
        let v1 = SemVer::parse("1.1.2").unwrap();
        let v2 = SemVer::parse("1.1.1").unwrap();
        assert!(v1 > v2);
    }

    #[test]
    fn test_stable_vs_prerelease() {
        let stable = SemVer::parse("1.2.3").unwrap();
        let prerelease = SemVer::parse("1.2.3-alpha").unwrap();
        assert!(stable > prerelease);
    }

    #[test]
    fn test_equal() {
        let v1 = SemVer::parse("1.2.3").unwrap();
        let v2 = SemVer::parse("1.2.3").unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn test_display_simple() {
        let v = SemVer::parse("1.2.3").unwrap();
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn test_display_prerelease() {
        let v = SemVer::parse("1.0.0-alpha").unwrap();
        assert_eq!(v.to_string(), "1.0.0-alpha");
    }

    #[test]
    fn test_display_with_v_prefix() {
        let v = SemVer::parse("v2.0.1").unwrap();
        assert_eq!(v.to_string(), "2.0.1");
    }

    #[test]
    fn test_parse_empty_string() {
        assert!(SemVer::parse("").is_none());
    }

    #[test]
    fn test_parse_whitespace() {
        assert!(SemVer::parse("  ").is_none());
    }

    #[test]
    fn test_parse_non_numeric() {
        assert!(SemVer::parse("1.2.x").is_none());
    }

    #[test]
    fn test_ord_less_than() {
        let a = SemVer::parse("1.0.0").unwrap();
        let b = SemVer::parse("2.0.0").unwrap();
        assert!(a < b);
    }

    #[test]
    fn test_ord_greater_than() {
        let a = SemVer::parse("3.0.0").unwrap();
        let b = SemVer::parse("1.0.0").unwrap();
        assert!(a > b);
    }

    #[test]
    fn test_ord_equal() {
        let a = SemVer::parse("1.2.3").unwrap();
        let b = SemVer::parse("1.2.3").unwrap();
        assert!(a == b);
    }

    #[test]
    fn test_ord_prerelease_less_than_stable() {
        let pre = SemVer::parse("1.0.0-alpha").unwrap();
        let stable = SemVer::parse("1.0.0").unwrap();
        assert!(pre < stable);
    }
}
