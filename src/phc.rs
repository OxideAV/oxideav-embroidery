//! Brother PHC and PHB — the machine-side design formats. Decode
//! only.
//!
//! Layout per the workspace's staged documentation
//! (`docs/embroidery/phc/phc-observed-structure.md`, validated
//! against eight real files spanning three colour counts and eight
//! design sizes; see `docs/embroidery/provenance/06-phc-offset-rule.md`
//! and `07-corpus3-multiformat.md`):
//!
//! - `#PHC` magic + 4-byte ASCII version (`"0001"` in every sample),
//! - a 36-byte copyright/producer string at 0x08 (skipped unread by
//!   the vendor loader),
//! - fixed header body from 0x2C: file size, extents as s16 pairs,
//!   thumbnail geometry (width, height, stride), colour count,
//!   palette (Brother thread indices),
//! - thumbnails from 0x60, one per colour, `stride × height` bytes,
//! - a `163 + 2n` design record (kept raw here; only partly decoded
//!   upstream), then
//! - the PEC stitch stream at `0x60 + n×(stride×height) + 163 + 2n`
//!   (= `259 + 230n` at the standard 48×38 geometry), decoded until
//!   `0xFF`.
//!
//! **PHB** is PHC with a second 36-byte copyright string at 0x2C and
//! a design record 9 bytes longer — not a different format. Every
//! PHC body field appears in PHB shifted by +36 (body from 0x50,
//! thumbnails from 0x84), and the stitch stream sits at
//! `259 + 230n + 45`. The 9 extra bytes at the head of PHB's design
//! record read as a length-like u32 followed by the value 1 — the
//! shape a multi-design container needs for a design count, but the
//! staged sample holds one design, so that reading is *suggested,
//! not proven* (see [`PhbFile::design_count_hint`]).
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

/// A decoded PHB file — PHC's multi-design sibling. Identical body
/// layout shifted +36 by a second copyright string, and a design
/// record 9 bytes longer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhbFile {
    /// ASCII version string (observed: `"0003"`).
    pub version: String,
    /// The first 36-byte copyright/producer string at 0x08, trimmed.
    pub copyright: String,
    /// The second 36-byte copyright/producer string at 0x2C, trimmed
    /// (identical to the first in the staged samples).
    pub copyright2: String,
    /// Declared total file size (header 0x50).
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
    /// The raw `172 + 2n` design record preceding the stitch stream
    /// (PHC's record plus a 9-byte head).
    pub design_record: Vec<u8>,
    /// The second u32 at the head of the design record. The staged
    /// documentation reads it as a design count (the sample carries
    /// 1), but with only single-design samples that reading is
    /// **suggested, not proven** — treat as a hint.
    pub design_count_hint: u32,
    /// The stitch design decoded from the embedded PEC stream.
    pub design: Design,
}

/// Returns true when `data` starts with the `#PHC` signature.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 8 && &data[..4] == b"#PHC"
}

/// Returns true when `data` starts with the `#PHB` signature.
pub fn probe_phb(data: &[u8]) -> bool {
    data.len() >= 8 && &data[..4] == b"#PHB"
}

/// The shared PHC/PHB body, parsed relative to `base` (0x2C for
/// PHC, 0x50 for PHB) with a `163 + extra + 2n` design record.
struct Body {
    file_size: u32,
    extents: (i16, i16, i16, i16),
    thumb_width: u16,
    thumb_height: u16,
    thumb_stride: u8,
    palette: Vec<u8>,
    thumbnails: Vec<Vec<u8>>,
    design_record: Vec<u8>,
    design: Design,
}

fn decode_body(data: &[u8], base: usize, record_extra: usize, name: &'static str) -> Result<Body> {
    let thumbs_start = base + 0x34;
    if data.len() < thumbs_start {
        return Err(Error::UnexpectedEof { context: name });
    }
    let s16 = |o: usize| i16::from_le_bytes([data[o], data[o + 1]]);
    let file_size =
        u32::from_le_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
    let extents = (
        s16(base + 0x10),
        s16(base + 0x12),
        s16(base + 0x14),
        s16(base + 0x16),
    );
    let thumb_width = u16::from_le_bytes([data[base + 0x1C], data[base + 0x1D]]);
    let thumb_height = u16::from_le_bytes([data[base + 0x1E], data[base + 0x1F]]);
    let thumb_stride = data[base + 0x20];
    let colors = data[base + 0x21] as usize;
    if colors == 0 {
        return Err(Error::invalid(format!("{name} colour count is zero")));
    }
    if base + 0x23 + colors > thumbs_start {
        return Err(Error::invalid(format!(
            "{name} colour count {colors} overruns the header palette area"
        )));
    }
    let palette = data[base + 0x23..base + 0x23 + colors].to_vec();

    let thumb_len = thumb_stride as usize * thumb_height as usize;
    let thumbs_end = thumbs_start + colors * thumb_len;
    // Design record: 163 + 2n bytes (+9 for PHB); the stitch stream
    // follows.
    let stitch_offset = thumbs_end + 163 + record_extra + 2 * colors;
    if stitch_offset >= data.len() {
        return Err(Error::UnexpectedEof { context: name });
    }
    let mut thumbnails = Vec::with_capacity(colors);
    for i in 0..colors {
        let start = thumbs_start + i * thumb_len;
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

    Ok(Body {
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

fn copyright_string(data: &[u8], offset: usize) -> String {
    String::from_utf8_lossy(&data[offset..offset + 36])
        .trim_end_matches([' ', '\0'])
        .to_string()
}

/// Decodes a PHC file.
pub fn decode(data: &[u8]) -> Result<PhcFile> {
    if data.len() < 4 || &data[..4] != b"#PHC" {
        return Err(Error::BadMagic { expected: "PHC" });
    }
    if data.len() < 0x2C {
        return Err(Error::UnexpectedEof {
            context: "PHC header",
        });
    }
    let version = String::from_utf8_lossy(&data[4..8]).into_owned();
    let copyright = copyright_string(data, 0x08);
    let body = decode_body(data, 0x2C, 0, "PHC")?;
    Ok(PhcFile {
        version,
        copyright,
        file_size: body.file_size,
        extents: body.extents,
        thumb_width: body.thumb_width,
        thumb_height: body.thumb_height,
        thumb_stride: body.thumb_stride,
        palette: body.palette,
        thumbnails: body.thumbnails,
        design_record: body.design_record,
        design: body.design,
    })
}

/// Decodes a PHB file.
pub fn decode_phb(data: &[u8]) -> Result<PhbFile> {
    if data.len() < 4 || &data[..4] != b"#PHB" {
        return Err(Error::BadMagic { expected: "PHB" });
    }
    if data.len() < 0x50 {
        return Err(Error::UnexpectedEof {
            context: "PHB header",
        });
    }
    let version = String::from_utf8_lossy(&data[4..8]).into_owned();
    let copyright = copyright_string(data, 0x08);
    let copyright2 = copyright_string(data, 0x2C);
    let body = decode_body(data, 0x50, 9, "PHB")?;
    let design_count_hint = u32::from_le_bytes([
        body.design_record[4],
        body.design_record[5],
        body.design_record[6],
        body.design_record[7],
    ]);
    Ok(PhbFile {
        version,
        copyright,
        copyright2,
        file_size: body.file_size,
        extents: body.extents,
        thumb_width: body.thumb_width,
        thumb_height: body.thumb_height,
        thumb_stride: body.thumb_stride,
        palette: body.palette,
        thumbnails: body.thumbnails,
        design_record: body.design_record,
        design_count_hint,
        design: body.design,
    })
}

/// The documented closed-form PHC stitch-stream offset for the
/// standard 48×38 thumbnail geometry: `259 + 230 × colour_count`.
pub fn stitch_offset_for(colors: usize) -> usize {
    259 + 230 * colors
}

/// The documented closed-form PHB stitch-stream offset for the
/// standard 48×38 thumbnail geometry: `259 + 230 × colour_count + 45`
/// (a PHB is exactly 45 bytes larger than its PHC sibling — one
/// 36-byte copyright string plus a 9-byte-longer design record).
pub fn phb_stitch_offset_for(colors: usize) -> usize {
    stitch_offset_for(colors) + 45
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

    /// Builds a synthetic PHB file per the staged layout: PHC with a
    /// second copyright string at 0x2C and a 9-byte design-record
    /// head carrying a length-like u32 then a design count of 1.
    fn synthesize_phb(palette: &[u8], design: &Design) -> Vec<u8> {
        let n = palette.len();
        let stitches = crate::pec::encode_stitches(design).unwrap();
        let e = design.extents();
        let mut out = Vec::new();
        out.extend_from_slice(b"#PHB");
        out.extend_from_slice(b"0003");
        out.extend_from_slice(format!("{:<36}", "Synthetic fixture, this crate").as_bytes());
        out.extend_from_slice(format!("{:<36}", "Synthetic fixture, this crate").as_bytes());
        assert_eq!(out.len(), 0x50);
        let total = phb_stitch_offset_for(n) + stitches.len();
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&((35 + 229 * n) as u32).to_le_bytes()); // 0x54
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&0x0004_0000u32.to_le_bytes());
        out.extend_from_slice(&(e.min_x as i16).to_le_bytes());
        out.extend_from_slice(&(e.max_x as i16).to_le_bytes());
        out.extend_from_slice(&(e.min_y as i16).to_le_bytes());
        out.extend_from_slice(&(e.max_y as i16).to_le_bytes());
        out.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF]);
        out.extend_from_slice(&48u16.to_le_bytes());
        out.extend_from_slice(&38u16.to_le_bytes());
        out.push(6);
        out.push(n as u8);
        out.push(0);
        out.extend_from_slice(palette);
        out.resize(0x84, 0);
        for _ in 0..n {
            out.extend_from_slice(&[0u8; THUMB_LEN]);
        }
        // 9-byte record head: length-like u32, design count 1, one
        // more raw byte — then the PHC-style 163 + 2n record body.
        let record_len = 163 + 2 * n;
        out.extend_from_slice(&(record_len as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.push(0);
        out.extend_from_slice(&vec![0u8; record_len]);
        assert_eq!(out.len(), phb_stitch_offset_for(n));
        out.extend_from_slice(&stitches);
        out
    }

    #[test]
    fn offset_rule_matches_validated_points() {
        // Validated at n = 5 → 0x581, n = 11 → 0xAE5 and n = 3 → 949.
        assert_eq!(stitch_offset_for(5), 0x581);
        assert_eq!(stitch_offset_for(11), 0xAE5);
        assert_eq!(stitch_offset_for(3), 949);
    }

    #[test]
    fn phb_offset_rule_matches_validated_point() {
        // Validated at n = 3 → 994 (all three staged size variants).
        assert_eq!(phb_stitch_offset_for(3), 994);
    }

    #[test]
    fn phb_decode_synthetic() {
        let d = sample();
        let bytes = synthesize_phb(&[14, 21, 20], &d);
        assert!(probe_phb(&bytes));
        assert!(!probe(&bytes));
        let f = decode_phb(&bytes).unwrap();
        assert_eq!(f.version, "0003");
        assert_eq!(f.copyright, f.copyright2);
        assert_eq!(f.file_size as usize, bytes.len());
        assert_eq!(f.palette, vec![14, 21, 20]);
        assert_eq!(f.thumbnails.len(), 3);
        assert_eq!(f.design_record.len(), 172 + 6);
        assert_eq!(f.design_count_hint, 1);
        assert_eq!(f.design.counts(), d.counts());
    }

    #[test]
    fn phb_is_exactly_45_bytes_larger_than_its_phc_sibling() {
        // The staged corpus finding, re-created with self-synthesized
        // files: same design, same palette, PHB = PHC + 45 bytes, and
        // the embedded PEC stitch streams are byte-identical.
        let d = sample();
        let phc = synthesize(&[14, 21, 20], &d);
        let phb = synthesize_phb(&[14, 21, 20], &d);
        assert_eq!(phb.len(), phc.len() + 45);
        assert_eq!(
            &phb[phb_stitch_offset_for(3)..],
            &phc[stitch_offset_for(3)..]
        );
        let a = decode(&phc).unwrap();
        let b = decode_phb(&phb).unwrap();
        assert_eq!(a.design.commands, b.design.commands);
        assert_eq!(a.extents, b.extents);
    }

    #[test]
    fn phb_truncated_rejected() {
        let d = sample();
        let mut bytes = synthesize_phb(&[1], &d);
        bytes.truncate(phb_stitch_offset_for(1) - 1);
        assert!(matches!(
            decode_phb(&bytes),
            Err(Error::UnexpectedEof { .. })
        ));
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
