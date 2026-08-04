//! Compucon / Singer XXX — header/metadata parsing only.
//!
//! Per the workspace's staged documentation (`docs/embroidery/xxx/`,
//! read out of real files; no third-party description exists): a
//! fixed-size **256-byte header**, largely zero-filled, with the
//! colour count at `0x27` (validated against every sibling of the
//! staged corpus design), followed by stitch data from `0x100`.
//!
//! The staged material observes that the stitch records are
//! *consistent with* 2-byte signed delta pairs plus escape forms,
//! but the control vocabulary was not established, a palette-like
//! run in the header tail is unmapped, and how the trailing RGB
//! data relates to the colour count is open. So this module
//! surfaces the header facts and the raw sections only;
//! [`decode_design`] returns [`Error::Unsupported`] until the docs
//! collaborator stages the record vocabulary.
//!
//! XXX has **no magic/signature** — the header opens with zeros —
//! so it cannot be probed from content and never appears in the
//! crate-level dispatch. Use [`parse`] directly when the extension
//! says `.xxx` (the same arrangement as headerless Melco EXP).

use crate::{Design, Error, Result};

/// The fixed header length.
pub const HEADER_LEN: usize = 0x100;

/// Offset of the colour-count byte inside the header.
pub const COLOR_COUNT_OFFSET: usize = 0x27;

/// A parsed XXX file: the documented header facts plus the raw
/// sections everything else lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XxxFile {
    /// Colour count (header byte `0x27`).
    pub color_count: u8,
    /// The raw 256-byte header (fields beyond the colour count are
    /// unmapped upstream).
    pub header: Vec<u8>,
    /// Raw stitch data from `0x100` to end of file (includes the
    /// trailing RGB area, whose boundary is undocumented).
    pub stitch_data: Vec<u8>,
}

/// Parses an XXX file's documented structure.
///
/// There is no signature to verify, so this can only check the
/// structural minimum: the file must be at least one full header
/// long. Callers should gate on the `.xxx` extension.
pub fn parse(data: &[u8]) -> Result<XxxFile> {
    if data.len() < HEADER_LEN {
        return Err(Error::UnexpectedEof {
            context: "XXX header",
        });
    }
    Ok(XxxFile {
        color_count: data[COLOR_COUNT_OFFSET],
        header: data[..HEADER_LEN].to_vec(),
        stitch_data: data[HEADER_LEN..].to_vec(),
    })
}

/// Stitch decoding — blocked on the unestablished record control
/// vocabulary; always returns [`Error::Unsupported`].
pub fn decode_design(_file: &XxxFile) -> Result<Design> {
    Err(Error::Unsupported {
        what: "XXX stitch-record control vocabulary is not established by the staged documentation",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic() -> Vec<u8> {
        let mut out = vec![0u8; HEADER_LEN];
        out[COLOR_COUNT_OFFSET] = 5;
        // Delta-pair-shaped bytes per the staged observation; their
        // meaning is not decoded, so they are opaque payload here.
        out.extend_from_slice(&[0x02, 0x05, 0xFE, 0xFB, 0x02, 0x05]);
        out
    }

    #[test]
    fn parse_synthetic() {
        let bytes = synthetic();
        let f = parse(&bytes).unwrap();
        assert_eq!(f.color_count, 5);
        assert_eq!(f.header.len(), HEADER_LEN);
        assert_eq!(f.stitch_data, vec![0x02, 0x05, 0xFE, 0xFB, 0x02, 0x05]);
        assert!(matches!(decode_design(&f), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn header_only_file_parses_with_empty_stitch_data() {
        let f = parse(&vec![0u8; HEADER_LEN]).unwrap();
        assert_eq!(f.color_count, 0);
        assert!(f.stitch_data.is_empty());
    }

    #[test]
    fn short_file_rejected() {
        assert!(matches!(
            parse(&[0u8; HEADER_LEN - 1]),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}
