//! Husqvarna Viking HUS and VIP — header/metadata parsing only.
//!
//! Per the workspace's staged documentation (`docs/embroidery/hus/`,
//! header mapped from real files and validated against two designs):
//! a little-endian header with the magic `0x00C8AF5B`, stitch count,
//! colour count, s16 extents, offsets to three **separate compressed
//! streams** (attributes, X deltas, Y deltas) and a u16 colour table
//! at 0x28.
//!
//! **VIP** shares the HUS header layout field for field, differing
//! only in its magic (`0x0190FC5D`) and in inserting 44 bytes ahead
//! of the block area (the three stored stream offsets are uniformly
//! 44 higher, so offset-directed parsing works unchanged; the
//! inserted bytes are unmapped upstream and surfaced raw here).
//!
//! The compression scheme of the three streams is not documented by
//! the staged material — for either format — so stitch decode is
//! unavailable; these parsers surface everything the header states
//! plus the raw compressed streams, and [`decode_design`] returns
//! [`Error::Unsupported`] until the docs collaborator stages the
//! bitstream.

use crate::model::Thread;
use crate::{Design, Error, Result};

/// The HUS magic (u32 LE at offset 0).
pub const MAGIC: u32 = 0x00C8_AF5B;

/// The VIP magic (u32 LE at offset 0).
pub const VIP_MAGIC: u32 = 0x0190_FC5D;

/// A parsed HUS or VIP file: header metadata plus the raw
/// compressed streams (the two formats share this layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HusFile {
    /// Declared stitch/record count.
    pub stitch_count: u32,
    /// Extents: (+X, +Y, −X, −Y) as stored, 0.1 mm.
    pub extents: (i16, i16, i16, i16),
    /// Husqvarna thread indices, one per colour.
    pub palette: Vec<u16>,
    /// Raw compressed attribute (command) stream.
    pub attributes: Vec<u8>,
    /// Raw compressed X-delta stream.
    pub x_deltas: Vec<u8>,
    /// Raw compressed Y-delta stream.
    pub y_deltas: Vec<u8>,
    /// Offset of the string/label area (0 when absent).
    pub string_offset: u32,
    /// Raw bytes between the end of the colour table and the first
    /// stream offset. Empty or small padding in HUS; in VIP this
    /// carries the documented 44 inserted bytes, unmapped upstream.
    pub unmapped: Vec<u8>,
}

impl HusFile {
    /// Threads derived from the palette (vendor indices only; the
    /// staged docs carry no Husqvarna index → colour table).
    pub fn threads(&self) -> Vec<Thread> {
        self.palette
            .iter()
            .map(|&idx| Thread {
                palette_index: Some(idx),
                ..Default::default()
            })
            .collect()
    }
}

/// Returns true when `data` starts with the HUS magic.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 4 && u32::from_le_bytes([data[0], data[1], data[2], data[3]]) == MAGIC
}

/// Returns true when `data` starts with the VIP magic.
pub fn probe_vip(data: &[u8]) -> bool {
    data.len() >= 4 && u32::from_le_bytes([data[0], data[1], data[2], data[3]]) == VIP_MAGIC
}

/// Parses a HUS header and splits out the raw compressed streams.
pub fn parse(data: &[u8]) -> Result<HusFile> {
    parse_with_magic(data, MAGIC, "HUS")
}

/// Parses a VIP header and splits out the raw compressed streams.
/// The layout is HUS's; the stored stream offsets already account
/// for VIP's 44 inserted bytes, which land in [`HusFile::unmapped`].
pub fn parse_vip(data: &[u8]) -> Result<HusFile> {
    parse_with_magic(data, VIP_MAGIC, "VIP")
}

fn parse_with_magic(data: &[u8], magic: u32, name: &'static str) -> Result<HusFile> {
    if data.len() < 0x28 {
        return Err(Error::UnexpectedEof { context: name });
    }
    let u32le = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let s16 = |o: usize| i16::from_le_bytes([data[o], data[o + 1]]);
    if u32le(0) != magic {
        return Err(Error::BadMagic { expected: name });
    }
    let stitch_count = u32le(4);
    let colors = u32le(8) as usize;
    let extents = (s16(0x0C), s16(0x0E), s16(0x10), s16(0x12));
    let attr_off = u32le(0x14) as usize;
    let x_off = u32le(0x18) as usize;
    let y_off = u32le(0x1C) as usize;
    let string_offset = u32le(0x20);
    if colors > 0x100 {
        return Err(Error::invalid(format!("{name} colour count {colors}")));
    }
    let table_end = 0x28 + 2 * colors;
    if table_end > data.len() {
        return Err(Error::UnexpectedEof { context: name });
    }
    let palette: Vec<u16> = (0..colors)
        .map(|i| u16::from_le_bytes([data[0x28 + 2 * i], data[0x28 + 2 * i + 1]]))
        .collect();
    if !(table_end <= attr_off && attr_off <= x_off && x_off <= y_off && y_off <= data.len()) {
        return Err(Error::invalid(format!(
            "{name} stream offsets are not in ascending order inside the file"
        )));
    }
    Ok(HusFile {
        stitch_count,
        extents,
        palette,
        attributes: data[attr_off..x_off].to_vec(),
        x_deltas: data[x_off..y_off].to_vec(),
        y_deltas: data[y_off..].to_vec(),
        string_offset,
        unmapped: data[table_end..attr_off].to_vec(),
    })
}

/// Stitch decoding — blocked on the undocumented compression of the
/// three streams (shared by HUS and VIP); always returns
/// [`Error::Unsupported`].
pub fn decode_design(_file: &HusFile) -> Result<Design> {
    Err(Error::Unsupported {
        what: "HUS/VIP stitch streams use a compression scheme the staged documentation does not cover",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic() -> Vec<u8> {
        let palette = [0u16, 26, 6];
        let attr = [1u8, 2, 3];
        let xs = [4u8, 5];
        let ys = [6u8, 7, 8, 9];
        let header_len = 0x28 + 2 * palette.len();
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC.to_le_bytes());
        out.extend_from_slice(&55184u32.to_le_bytes());
        out.extend_from_slice(&(palette.len() as u32).to_le_bytes());
        for v in [647i16, 907, -648, -908] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&(header_len as u32).to_le_bytes());
        out.extend_from_slice(&((header_len + attr.len()) as u32).to_le_bytes());
        out.extend_from_slice(&((header_len + attr.len() + xs.len()) as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for p in palette {
            out.extend_from_slice(&p.to_le_bytes());
        }
        out.extend_from_slice(&attr);
        out.extend_from_slice(&xs);
        out.extend_from_slice(&ys);
        out
    }

    /// Same design as [`synthetic`], laid out as a VIP: the magic
    /// differs and 44 bytes are inserted ahead of the block area, so
    /// every stream offset is 44 higher — the staged corpus finding.
    fn synthetic_vip() -> Vec<u8> {
        let mut out = synthetic();
        out[0..4].copy_from_slice(&VIP_MAGIC.to_le_bytes());
        let table_end = 0x28 + 2 * 3;
        for field in [0x14, 0x18, 0x1C] {
            let v =
                u32::from_le_bytes([out[field], out[field + 1], out[field + 2], out[field + 3]]);
            out[field..field + 4].copy_from_slice(&(v + 44).to_le_bytes());
        }
        for i in 0..44u8 {
            out.insert(table_end, 0xA0 + (i % 8));
        }
        out
    }

    #[test]
    fn parse_synthetic() {
        let bytes = synthetic();
        assert!(probe(&bytes));
        assert!(!probe_vip(&bytes));
        let f = parse(&bytes).unwrap();
        assert_eq!(f.stitch_count, 55184);
        assert_eq!(f.extents, (647, 907, -648, -908));
        assert_eq!(f.palette, vec![0, 26, 6]);
        assert_eq!(f.attributes, vec![1, 2, 3]);
        assert_eq!(f.x_deltas, vec![4, 5]);
        assert_eq!(f.y_deltas, vec![6, 7, 8, 9]);
        assert!(f.unmapped.is_empty());
        assert!(matches!(decode_design(&f), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn parse_synthetic_vip() {
        let bytes = synthetic_vip();
        assert!(probe_vip(&bytes));
        assert!(!probe(&bytes));
        // A VIP does not parse as a HUS and vice versa.
        assert!(matches!(parse(&bytes), Err(Error::BadMagic { .. })));
        let f = parse_vip(&bytes).unwrap();
        let h = parse(&synthetic()).unwrap();
        // Field-for-field agreement with the HUS twin, bar the
        // 44 inserted bytes.
        assert_eq!(f.stitch_count, h.stitch_count);
        assert_eq!(f.extents, h.extents);
        assert_eq!(f.palette, h.palette);
        assert_eq!(f.attributes, h.attributes);
        assert_eq!(f.x_deltas, h.x_deltas);
        assert_eq!(f.y_deltas, h.y_deltas);
        assert_eq!(f.unmapped.len(), 44);
        assert!(matches!(decode_design(&f), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn stream_offset_inside_color_table_rejected() {
        let mut bytes = synthetic();
        // Point the attribute stream inside the colour table.
        bytes[0x14..0x18].copy_from_slice(&0x10u32.to_le_bytes());
        assert!(matches!(parse(&bytes), Err(Error::Invalid { .. })));
    }

    #[test]
    fn bad_magic_rejected() {
        assert!(matches!(parse(&[0u8; 64]), Err(Error::BadMagic { .. })));
        assert!(matches!(parse_vip(&[0u8; 64]), Err(Error::BadMagic { .. })));
    }
}
