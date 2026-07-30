//! Husqvarna Viking / Pfaff VP3 — signature probe and header skim
//! only.
//!
//! Per the workspace's staged documentation (`docs/embroidery/vp3/`,
//! observed from real files): magic `%vsm%`, big-endian
//! length-prefixed fields, UTF-16BE strings opening with a producer
//! credit, and embedded thread *names*. The stitch encoding and the
//! section structure are open questions upstream, so this module
//! only identifies the format and surfaces the leading producer
//! string; [`decode_design`] returns [`Error::Unsupported`].

use crate::{Design, Error, Result};

/// The VP3 magic.
pub const MAGIC: &[u8; 5] = b"%vsm%";

/// The little that can be read from a VP3 header today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vp3File {
    /// The producer-credit string that opens the file, when it
    /// parses cleanly as a BE-length-prefixed UTF-16BE string.
    pub producer: Option<String>,
}

/// Returns true when `data` starts with the `%vsm%` magic.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= MAGIC.len() && &data[..MAGIC.len()] == MAGIC
}

/// Parses the VP3 opening: magic + the leading UTF-16BE string.
pub fn parse(data: &[u8]) -> Result<Vp3File> {
    if !probe(data) {
        return Err(Error::BadMagic { expected: "VP3" });
    }
    let mut producer = None;
    // A u16 BE byte-length precedes string data; the first string is
    // the producer credit.
    if let Some(len_bytes) = data.get(5..7) {
        let len = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
        if len % 2 == 0 {
            if let Some(raw) = data.get(7..7 + len) {
                let units: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                if let Ok(s) = String::from_utf16(&units) {
                    // Accept only plausible text.
                    if !s.is_empty()
                        && s.chars().all(|c| !c.is_control())
                        && s.chars().any(|c| c.is_ascii_alphabetic())
                    {
                        producer = Some(s);
                    }
                }
            }
        }
    }
    Ok(Vp3File { producer })
}

/// Stitch decoding — the VP3 stitch section is not covered by the
/// staged documentation; always returns [`Error::Unsupported`].
pub fn decode_design(_file: &Vp3File) -> Result<Design> {
    Err(Error::Unsupported {
        what: "VP3 stitch section is not covered by the staged documentation",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic() -> Vec<u8> {
        let text = "Produced by a synthetic fixture";
        let mut out = MAGIC.to_vec();
        let utf16: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        out.extend_from_slice(&(utf16.len() as u16).to_be_bytes());
        out.extend_from_slice(&utf16);
        out.extend_from_slice(&[0u8; 32]);
        out
    }

    #[test]
    fn probe_and_parse() {
        let bytes = synthetic();
        assert!(probe(&bytes));
        let f = parse(&bytes).unwrap();
        assert_eq!(
            f.producer.as_deref(),
            Some("Produced by a synthetic fixture")
        );
        assert!(matches!(decode_design(&f), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn garbage_string_yields_none() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&4u16.to_be_bytes());
        bytes.extend_from_slice(&[0x00, 0x01, 0x00, 0x02]); // control chars
        let f = parse(&bytes).unwrap();
        assert_eq!(f.producer, None);
    }

    #[test]
    fn bad_magic_rejected() {
        assert!(matches!(parse(b"nope"), Err(Error::BadMagic { .. })));
    }
}
