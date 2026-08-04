//! Bernina ART — extension recognition and vendor taxonomy only.
//!
//! Per the workspace's staged documentation (`docs/embroidery/art/`,
//! first-party vendor statements, staged 2026-07-31): ART is
//! Bernina's native design format — an "all-in-one" design file
//! carrying object properties, object outlines, stitch types, stitch
//! coordinates, stitch data, thread colours, auto-spacing and
//! pull-compensation settings, comments and a thumbnail. Files are
//! **compressed on save**. The same extension also serves as a
//! machine transfer format for certain hardware (A730/A200).
//!
//! The staged material is **vendor statements about contents only**:
//! no magic, no container layout, no compression scheme, no object
//! model and no stitch storage are documented, and no sample is
//! held (`docs/embroidery/art/GAP-TRACKER.md` — a total structural
//! gap). Consequently this module can offer **no content probe and
//! no parsing at all** — only the documented extension family, whose
//! numbered variants carry the producing software version in the
//! extension itself.

use crate::{Design, Error, Result};

/// The documented ART extension family, lower-case: bare `.art`
/// plus the numbered variants produced as the vendor's design
/// software advanced.
pub const EXTENSIONS: [&str; 7] = ["art", "art42", "art50", "art60", "art70", "art80", "art90"];

/// Returns true when `ext` (with or without a leading dot, any
/// case) is a documented ART extension.
pub fn is_art_extension(ext: &str) -> bool {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    EXTENSIONS.contains(&e.as_str())
}

/// The software version a numbered ART extension carries, as
/// `(major, minor)` — the version number is stored in the extension
/// rather than inside the file (vendor statement): `.art42` → (4, 2),
/// `.art50` → (5, 0), … `.art90` → (9, 0). Bare `.art` carries no
/// version and yields `None`.
pub fn version_hint(ext: &str) -> Option<(u8, u8)> {
    let e = ext.trim_start_matches('.').to_ascii_lowercase();
    if !EXTENSIONS.contains(&e.as_str()) {
        return None;
    }
    let digits = e.strip_prefix("art")?;
    let mut chars = digits.chars();
    let major = chars.next()?.to_digit(10)? as u8;
    let minor = chars.next()?.to_digit(10)? as u8;
    Some((major, minor))
}

/// Decoding — blocked on the total structural gap: the staged
/// documentation records what an ART file *contains*, never how a
/// single byte of it is laid out. Always returns
/// [`Error::Unsupported`].
pub fn decode_design(_data: &[u8]) -> Result<Design> {
    Err(Error::Unsupported {
        what: "ART structure is entirely undocumented by the staged material (no magic, container, compression, object model or stitch storage)",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_family_recognised() {
        for ext in ["art", ".art", "ART42", ".Art90", "art50"] {
            assert!(is_art_extension(ext), "{ext}");
        }
        for ext in ["art10", "artx", "dst", "", ".art100"] {
            assert!(!is_art_extension(ext), "{ext}");
        }
    }

    #[test]
    fn version_hints_follow_the_extension() {
        assert_eq!(version_hint(".art42"), Some((4, 2)));
        assert_eq!(version_hint("art50"), Some((5, 0)));
        assert_eq!(version_hint("ART90"), Some((9, 0)));
        assert_eq!(version_hint("art"), None);
        assert_eq!(version_hint("jef"), None);
    }

    #[test]
    fn decode_is_unsupported() {
        assert!(matches!(
            decode_design(&[0u8; 64]),
            Err(Error::Unsupported { .. })
        ));
    }
}
