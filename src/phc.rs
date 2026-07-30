//! Brother PHC — the machine-side design format. Decode only.
//!
//! Layout per the workspace's staged documentation
//! (`docs/embroidery/phc/phc-observed-structure.md`, validated
//! against five real files spanning two colour counts and five
//! design sizes; see `docs/embroidery/provenance/06-phc-offset-rule.md`):
//!
//! - `#PHC` magic + 4-byte ASCII version (`"0001"` in every sample),
//! - a 36-byte copyright/producer string at 0x08 (skipped unread by
//!   the vendor loader),
//! - fixed header: file size at 0x2C, extents as s16 pairs at
//!   0x3C/0x40, thumbnail geometry at 0x48 (width, height, stride),
//!   colour count at 0x4D, palette (Brother thread indices) at 0x4F,
//! - thumbnails from 0x60, one per colour, `stride × height` bytes,
//! - a `163 + 2n` design record (kept raw here; only partly decoded
//!   upstream), then
//! - the PEC stitch stream at `0x60 + n×(stride×height) + 163 + 2n`
//!   (= `259 + 230n` at the standard 48×38 geometry), decoded until
//!   `0xFF`.
//!
//! Encoding is not offered: the design record's 163-byte fixed part
//! is not decoded by the staged documentation, so a faithful writer
//! cannot be built yet.

use crate::model::{Design, Thread};
use crate::pec;
use crate::{Error, Result};

/// A decoded PHC file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhcFile {
    /// ASCII version string (observed: `"0001"`).
    pub version: String,
    /// The 36-byte copyright/producer string at 0x08, trimmed.
    pub copyright: String,
    /// Declared total file size (header 0x2C).
    pub file_size: u32,
    /// Extents from the header: (−X, +X, −Y, +Y), 0.1 mm.
    pub extents: (i16, i16, i16, i16),
    /// Thumbnail width in pixels.
    pub thumb_width: u16,
    /// Thumbnail height in pixels.
    pub thumb_height: u16,
    /// Thumbnail row stride in bytes.
    pub thumb_stride: u8,
    /// Brother thread-table indices, one per colour.
    pub palette: Vec<u8>,
    /// Raw 1-bit thumbnails, one per colour (the first carries the
    /// framed overview).
    pub thumbnails: Vec<Vec<u8>>,
    /// The raw `163 + 2n` design record preceding the stitch stream.
    pub design_record: Vec<u8>,
    /// The stitch design decoded from the embedded PEC stream.
    pub design: Design,
}

/// Returns true when `data` starts with the `#PHC` signature.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 8 && &data[..4] == b"#PHC"
}

/// Decodes a PHC file.
pub fn decode(data: &[u8]) -> Result<PhcFile> {
    if data.len() < 4 || &data[..4] != b"#PHC" {
        return Err(Error::BadMagic { expected: "PHC" });
    }
    if data.len() < 0x60 {
        return Err(Error::UnexpectedEof {
            context: "PHC header",
        });
    }
    let version = String::from_utf8_lossy(&data[4..8]).into_owned();
    let copyright = String::from_utf8_lossy(&data[0x08..0x2C])
        .trim_end_matches([' ', '\0'])
        .to_string();
    let file_size = u32::from_le_bytes([data[0x2C], data[0x2D], data[0x2E], data[0x2F]]);
    let s16 = |o: usize| i16::from_le_bytes([data[o], data[o + 1]]);
    let extents = (s16(0x3C), s16(0x3E), s16(0x40), s16(0x42));
    let thumb_width = u16::from_le_bytes([data[0x48], data[0x49]]);
    let thumb_height = u16::from_le_bytes([data[0x4A], data[0x4B]]);
    let thumb_stride = data[0x4C];
    let colors = data[0x4D] as usize;
    if colors == 0 {
        return Err(Error::invalid("PHC colour count is zero"));
    }
    if 0x4F + colors > 0x60 {
        return Err(Error::invalid(format!(
            "PHC colour count {colors} overruns the header palette area"
        )));
    }
    let palette = data[0x4F..0x4F + colors].to_vec();

    let thumb_len = thumb_stride as usize * thumb_height as usize;
    let thumbs_end = 0x60 + colors * thumb_len;
    // Design record: 163 + 2n bytes; the stitch stream follows.
    let stitch_offset = thumbs_end + 163 + 2 * colors;
    if stitch_offset >= data.len() {
        return Err(Error::UnexpectedEof {
            context: "PHC stitch stream",
        });
    }
    let mut thumbnails = Vec::with_capacity(colors);
    for i in 0..colors {
        let start = 0x60 + i * thumb_len;
        thumbnails.push(data[start..start + thumb_len].to_vec());
    }
    let design_record = data[thumbs_end..stitch_offset].to_vec();

    let (commands, _leading, _used) = pec::decode_stitches(&data[stitch_offset..])?;
    let threads = palette
        .iter()
        .map(|&idx| {
            let known = pec::brother_thread(idx);
            Thread {
                palette_index: Some(idx as u16),
                rgb: known.map(|k| k.rgb),
                catalog: known.map(|k| k.code.to_string()),
                name: known.map(|k| k.name.to_string()),
            }
        })
        .collect();

    Ok(PhcFile {
        version,
        copyright,
        file_size,
        extents,
        thumb_width,
        thumb_height,
        thumb_stride,
        palette,
        thumbnails,
        design_record,
        design: Design {
            commands,
            threads,
            label: None,
        },
    })
}

/// The documented closed-form stitch-stream offset for the standard
/// 48×38 thumbnail geometry: `259 + 230 × colour_count`.
pub fn stitch_offset_for(colors: usize) -> usize {
    259 + 230 * colors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Design};
    use crate::pec::THUMB_LEN;

    /// Builds a synthetic PHC file per the staged layout. No corpus
    /// bytes are embedded: the stitch stream comes from this crate's
    /// own PEC encoder and every header field is computed.
    fn synthesize(palette: &[u8], design: &Design) -> Vec<u8> {
        let n = palette.len();
        let stitches = crate::pec::encode_stitches(design).unwrap();
        let e = design.extents();
        let mut out = Vec::new();
        out.extend_from_slice(b"#PHC");
        out.extend_from_slice(b"0001");
        out.extend_from_slice(format!("{:<36}", "Synthetic fixture, this crate").as_bytes());
        assert_eq!(out.len(), 0x2C);
        let total = stitch_offset_for(n) + stitches.len();
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&((35 + 229 * n) as u32).to_le_bytes()); // 0x30
        out.extend_from_slice(&2u32.to_le_bytes()); // 0x34 (observed constant)
        out.extend_from_slice(&0x0004_0000u32.to_le_bytes()); // 0x38 (observed constant)
        out.extend_from_slice(&(e.min_x as i16).to_le_bytes());
        out.extend_from_slice(&(e.max_x as i16).to_le_bytes());
        out.extend_from_slice(&(e.min_y as i16).to_le_bytes());
        out.extend_from_slice(&(e.max_y as i16).to_le_bytes());
        out.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF]); // 0x44 constant
        out.extend_from_slice(&48u16.to_le_bytes());
        out.extend_from_slice(&38u16.to_le_bytes());
        out.push(6);
        out.push(n as u8);
        out.push(0);
        out.extend_from_slice(palette);
        out.resize(0x60, 0);
        for _ in 0..n {
            out.extend_from_slice(&[0u8; THUMB_LEN]);
        }
        // Raw design record (not decoded by this crate): zero-filled
        // at the documented 163 + 2n size.
        out.extend_from_slice(&vec![0u8; 163 + 2 * n]);
        assert_eq!(out.len(), stitch_offset_for(n));
        out.extend_from_slice(&stitches);
        out
    }

    fn sample() -> Design {
        Design {
            commands: vec![
                Command::Jump { dx: 40, dy: 40 },
                Command::Stitch { dx: 12, dy: 0 },
                Command::Trim { dx: 0, dy: 0 },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::Stitch { dx: -12, dy: -6 },
                Command::End,
            ],
            ..Default::default()
        }
    }

    #[test]
    fn offset_rule_matches_validated_points() {
        // Validated at n = 5 → 0x581 and n = 11 → 0xAE5.
        assert_eq!(stitch_offset_for(5), 0x581);
        assert_eq!(stitch_offset_for(11), 0xAE5);
    }

    #[test]
    fn decode_synthetic() {
        let d = sample();
        let bytes = synthesize(&[20, 5], &d);
        assert!(probe(&bytes));
        let f = decode(&bytes).unwrap();
        assert_eq!(f.version, "0001");
        assert_eq!(f.copyright, "Synthetic fixture, this crate");
        assert_eq!(f.file_size as usize, bytes.len());
        assert_eq!(f.palette, vec![20, 5]);
        assert_eq!(f.thumbnails.len(), 2);
        assert_eq!(f.design_record.len(), 163 + 4);
        assert_eq!(f.design.counts(), d.counts());
        let e = d.extents();
        assert_eq!(f.extents.0 as i32, e.min_x);
        assert_eq!(f.extents.1 as i32, e.max_x);
        assert_eq!(f.design.threads[0].name.as_deref(), Some("Black"));
    }

    #[test]
    fn stitch_stream_agrees_with_pec_sibling() {
        // The corpus-validated headline: a PHC embeds the identical
        // PEC stitch stream its .pes sibling carries. Re-created here
        // with self-synthesized files of the same design.
        let d = sample();
        let phc = synthesize(&[1, 2], &d);
        let pes = crate::pes::encode(&d, &Default::default()).unwrap();
        let phc_stream = &phc[stitch_offset_for(2)..];
        let pes_file = crate::pes::decode(&pes).unwrap();
        let phc_file = decode(&phc).unwrap();
        assert_eq!(phc_file.design.commands, pes_file.pec.design.commands);
        // Byte-level identity of the embedded stream.
        let pec_block = crate::pec::encode_block(&d, &Default::default()).unwrap();
        let pes_stream = &pec_block[crate::pec::STITCH_OFFSET..];
        assert!(pes_stream.starts_with(phc_stream));
    }

    #[test]
    fn truncated_rejected() {
        let d = sample();
        let mut bytes = synthesize(&[1], &d);
        bytes.truncate(stitch_offset_for(1) - 1);
        assert!(matches!(decode(&bytes), Err(Error::UnexpectedEof { .. })));
    }

    #[test]
    fn zero_colors_rejected() {
        let d = sample();
        let mut bytes = synthesize(&[1], &d);
        bytes[0x4D] = 0;
        assert!(matches!(decode(&bytes), Err(Error::Invalid { .. })));
    }
}
