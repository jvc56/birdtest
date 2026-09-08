//! Semantic versions as three integers.
//!
//! MAGPIE versions are compared in two places -- the per-job floor and the
//! global one -- and both compare wrongly if the comparison is lexical:
//! `'1.10.0' < '1.9.0'` as text. The bug is invisible until a minor version
//! reaches double digits, which is precisely when nobody is looking at this
//! code any more, so versions are integers everywhere and only rendered as
//! text at the edges.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: i32,
    pub minor: i32,
    pub patch: i32,
}

impl Version {
    pub const ZERO: Version = Version { major: 0, minor: 0, patch: 0 };

    pub fn new(major: i32, minor: i32, patch: i32) -> Self {
        Self { major, minor, patch }
    }

    /// Parses `major.minor.patch`, treating anything unparseable as `0.0.0`.
    ///
    /// A client that reports a version this cannot read is offered nothing,
    /// because every job's floor is at least `0.0.1` -- which is the correct
    /// outcome and needs no special case. Trailing pre-release or build
    /// metadata (`1.4.0-rc1`) is ignored rather than rejected.
    pub fn parse_or_zero(text: &str) -> Self {
        let core = text.trim().split(['-', '+']).next().unwrap_or("");
        let mut parts = core.split('.');
        let mut next = || -> Option<i32> { parts.next()?.trim().parse().ok() };
        match (next(), next()) {
            (Some(major), Some(minor)) => {
                Self { major, minor, patch: next().unwrap_or(0) }
            }
            // "1" alone is not a version anyone means to send.
            _ => Self::ZERO,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_numerically_not_lexically() {
        // The whole reason this type exists: as text, "1.10.0" < "1.9.0".
        assert!(Version::parse_or_zero("1.10.0") > Version::parse_or_zero("1.9.0"));
        assert!(Version::parse_or_zero("2.0.0") > Version::parse_or_zero("1.99.99"));
        assert!(Version::parse_or_zero("1.4.10") > Version::parse_or_zero("1.4.9"));
        assert_eq!(Version::parse_or_zero("1.4.0"), Version::new(1, 4, 0));
    }

    #[test]
    fn unparseable_is_zero() {
        for text in ["", "nonsense", "1", "v1.4.0", "1.x.0"] {
            assert_eq!(Version::parse_or_zero(text), Version::ZERO, "{text:?}");
        }
        // A floor of 0.0.1 means zero is offered nothing.
        assert!(Version::ZERO < Version::new(0, 0, 1));
    }

    #[test]
    fn ignores_prerelease_and_build_metadata() {
        assert_eq!(Version::parse_or_zero("1.4.0-rc1"), Version::new(1, 4, 0));
        assert_eq!(Version::parse_or_zero("1.4.0+build7"), Version::new(1, 4, 0));
    }
}
