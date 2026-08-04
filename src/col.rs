//! The `.col` plain-text colour companion — decode + encode.
//!
//! Per the workspace's staged documentation
//! (`docs/embroidery/corpus-map.md`, identified from real files and
//! cross-validated against the `.inf` companion of the same design,
//! `docs/embroidery/provenance/07-corpus3-multiformat.md`): a
//! plain-text colour list — a count line, then one
//! `index,R,G,B` line per colour, CRLF-terminated:
//!
//! ```text
//! 3
//! 1,0,254,0
//! 2,0,28,223
//! 3,0,0,0
//! ```
//!
//! The RGB values in the staged sample match its `.inf` sibling
//! exactly, so `.col` and `.inf` are two encodings of the same
//! palette. Indices are 1-based in the observed file.

use crate::model::Thread;
use crate::{Error, Result};

/// Decodes a `.col` colour list into threads (index + RGB).
///
/// The observed file uses CRLF line endings and a trailing newline;
/// bare LF and a missing trailing newline are tolerated.
pub fn decode(data: &[u8]) -> Result<Vec<Thread>> {
    let text = core::str::from_utf8(data)
        .map_err(|_| Error::invalid("COL file is not ASCII/UTF-8 text"))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let count: usize = lines
        .next()
        .ok_or(Error::UnexpectedEof {
            context: "COL count line",
        })?
        .trim()
        .parse()
        .map_err(|_| Error::invalid("COL count line is not a number"))?;
    let mut threads = Vec::new();
    for line in lines {
        let mut fields = line.trim().split(',');
        let mut next = |what: &'static str| -> Result<u16> {
            fields
                .next()
                .ok_or_else(|| Error::invalid(format!("COL record is missing the {what} field")))?
                .trim()
                .parse::<u16>()
                .map_err(|_| Error::invalid(format!("COL {what} field is not a number")))
        };
        let index = next("index")?;
        let mut rgb = [0u8; 3];
        for (slot, what) in rgb.iter_mut().zip(["R", "G", "B"]) {
            let v = next(what)?;
            *slot = u8::try_from(v).map_err(|_| Error::OutOfRange {
                field: "COL colour component",
            })?;
        }
        if fields.next().is_some() {
            return Err(Error::invalid("COL record has more than four fields"));
        }
        threads.push(Thread {
            palette_index: Some(index),
            rgb: Some(rgb),
            catalog: None,
            name: None,
        });
    }
    if threads.len() != count {
        return Err(Error::invalid(format!(
            "COL count line says {count} colours but {} records follow",
            threads.len()
        )));
    }
    Ok(threads)
}

/// Encodes threads as a `.col` colour list (CRLF line endings, as
/// observed). Missing indices fall back to 1-based order; missing
/// RGB encodes as black.
pub fn encode(threads: &[Thread]) -> Result<Vec<u8>> {
    let mut out = format!("{}\r\n", threads.len());
    for (i, t) in threads.iter().enumerate() {
        let index = t.palette_index.unwrap_or(i as u16 + 1);
        let rgb = t.rgb.unwrap_or([0, 0, 0]);
        out.push_str(&format!("{},{},{},{}\r\n", index, rgb[0], rgb[1], rgb[2]));
    }
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_documented_shape() {
        // The staged sample's structure (count line, then
        // index,R,G,B records, CRLF).
        let bytes = b"3\r\n1,0,254,0\r\n2,0,28,223\r\n3,0,0,0\r\n";
        let threads = decode(bytes).unwrap();
        assert_eq!(threads.len(), 3);
        assert_eq!(threads[0].palette_index, Some(1));
        assert_eq!(threads[0].rgb, Some([0, 254, 0]));
        assert_eq!(threads[1].rgb, Some([0, 28, 223]));
        assert_eq!(threads[2].rgb, Some([0, 0, 0]));
    }

    #[test]
    fn tolerates_lf_and_missing_trailing_newline() {
        let threads = decode(b"2\n1,10,20,30\n2,40,50,60").unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[1].rgb, Some([40, 50, 60]));
    }

    #[test]
    fn roundtrip() {
        let threads = vec![
            Thread {
                palette_index: Some(1),
                rgb: Some([0, 254, 0]),
                ..Default::default()
            },
            Thread {
                palette_index: Some(2),
                rgb: Some([0, 28, 223]),
                ..Default::default()
            },
        ];
        let bytes = encode(&threads).unwrap();
        assert_eq!(bytes, b"2\r\n1,0,254,0\r\n2,0,28,223\r\n");
        assert_eq!(decode(&bytes).unwrap(), threads);
    }

    #[test]
    fn col_and_inf_agree_on_the_same_palette() {
        // The corpus finding: the RGBs in a .col match the .inf of
        // the same design exactly. Re-created with self-synthesized
        // companions carrying one palette.
        let threads = vec![
            Thread {
                palette_index: Some(1),
                rgb: Some([0, 254, 0]),
                ..Default::default()
            },
            Thread {
                palette_index: Some(2),
                rgb: Some([0, 0, 0]),
                ..Default::default()
            },
        ];
        let via_col = decode(&encode(&threads).unwrap()).unwrap();
        let via_inf = crate::exp::decode_inf(&crate::exp::encode_inf(&threads).unwrap()).unwrap();
        let rgb = |ts: &[Thread]| ts.iter().map(|t| t.rgb).collect::<Vec<_>>();
        assert_eq!(rgb(&via_col), rgb(&via_inf));
    }

    #[test]
    fn count_mismatch_rejected() {
        assert!(matches!(
            decode(b"3\r\n1,0,0,0\r\n"),
            Err(Error::Invalid { .. })
        ));
    }

    #[test]
    fn component_over_255_rejected() {
        assert!(matches!(
            decode(b"1\r\n1,256,0,0\r\n"),
            Err(Error::OutOfRange { .. })
        ));
    }

    #[test]
    fn non_text_rejected() {
        assert!(matches!(
            decode(&[0xFF, 0xFE, 0x00]),
            Err(Error::Invalid { .. })
        ));
    }
}
