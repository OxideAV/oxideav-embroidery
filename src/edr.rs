//! The `.edr` colour companion — decode + encode.
//!
//! Per the workspace's staged corpus findings
//! (`docs/embroidery/provenance/08-corpus-restage-and-gap-closure.md`):
//! an `.edr` file is `n` × 4-byte RGB records (R, G, B, `0x00`)
//! followed by a `FF FF FF 00` sentinel. The corpus's 3-colour
//! design decodes to exactly the RGB list its `.col` and `.inf`
//! siblings carry, line for line.
//!
//! Like [`crate::col`] and the `.inf` reader in [`crate::exp`], this
//! is a palette side-file that accompanies a stitch file of the same
//! stem; it stores colours only, no stitch data.

use crate::model::Thread;
use crate::{Error, Result};

/// The 4-byte end-of-list sentinel.
pub const SENTINEL: [u8; 4] = [0xFF, 0xFF, 0xFF, 0x00];

/// Decodes an `.edr` colour list. The sentinel is required; records
/// after it are rejected (no corpus file carries any).
pub fn decode(data: &[u8]) -> Result<Vec<Thread>> {
    if data.len() % 4 != 0 {
        return Err(Error::invalid(format!(
            "EDR length {} is not a whole number of 4-byte records",
            data.len()
        )));
    }
    let mut threads = Vec::new();
    let mut seen_sentinel = false;
    for (i, rec) in data.chunks_exact(4).enumerate() {
        if rec == SENTINEL {
            if 4 * (i + 1) != data.len() {
                return Err(Error::invalid("EDR data after the sentinel"));
            }
            seen_sentinel = true;
            break;
        }
        threads.push(Thread {
            palette_index: Some(i as u16 + 1),
            rgb: Some([rec[0], rec[1], rec[2]]),
            ..Default::default()
        });
    }
    if !seen_sentinel {
        return Err(Error::UnexpectedEof {
            context: "EDR sentinel",
        });
    }
    Ok(threads)
}

/// Encodes an `.edr` colour list for the given threads. A thread
/// without RGB encodes as black.
pub fn encode(threads: &[Thread]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 * threads.len() + 4);
    for t in threads {
        let rgb = t.rgb.unwrap_or([0, 0, 0]);
        out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0x00]);
    }
    out.extend_from_slice(&SENTINEL);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_worked_example() {
        // The corpus's 16-byte 3-colour file: (0,254,0), (0,28,223),
        // (0,0,0), sentinel.
        let bytes = [
            0x00, 0xFE, 0x00, 0x00, 0x00, 0x1C, 0xDF, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF,
            0xFF, 0x00,
        ];
        let threads = decode(&bytes).unwrap();
        assert_eq!(threads.len(), 3);
        assert_eq!(threads[0].rgb, Some([0, 254, 0]));
        assert_eq!(threads[1].rgb, Some([0, 28, 223]));
        assert_eq!(threads[2].rgb, Some([0, 0, 0]));
        assert_eq!(threads[0].palette_index, Some(1));
        assert_eq!(encode(&threads), bytes);
    }

    #[test]
    fn roundtrip() {
        let threads = vec![
            Thread {
                rgb: Some([1, 2, 3]),
                ..Default::default()
            },
            Thread {
                rgb: None, // encodes as black
                ..Default::default()
            },
        ];
        let bytes = encode(&threads);
        let got = decode(&bytes).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].rgb, Some([1, 2, 3]));
        assert_eq!(got[1].rgb, Some([0, 0, 0]));
    }

    #[test]
    fn missing_sentinel_rejected() {
        assert!(matches!(
            decode(&[1, 2, 3, 0]),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn data_after_sentinel_rejected() {
        let mut bytes = encode(&[]);
        bytes.extend_from_slice(&[9, 9, 9, 0]);
        assert!(matches!(decode(&bytes), Err(Error::Invalid { .. })));
    }

    #[test]
    fn ragged_length_rejected() {
        assert!(matches!(decode(&[1, 2, 3]), Err(Error::Invalid { .. })));
    }

    #[test]
    fn empty_list_roundtrips() {
        let bytes = encode(&[]);
        assert_eq!(bytes, SENTINEL);
        assert!(decode(&bytes).unwrap().is_empty());
    }
}
