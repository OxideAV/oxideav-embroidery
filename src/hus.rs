//! Husqvarna Viking HUS — header/metadata parsing only.
//!
//! Per the workspace's staged documentation (`docs/embroidery/hus/`,
//! header mapped from a real file): a little-endian header with the
//! magic `0x00C8AF5B`, stitch count, colour count, s16 extents,
//! offsets to three **separate compressed streams** (attributes,
//! X deltas, Y deltas) and a u16 colour table at 0x28.
//!
//! The compression scheme of the three streams is not documented by
//! the staged material, so stitch decode is unavailable; this module
//! surfaces everything the header states and the raw compressed
//! streams, and [`decode_design`] returns [`Error::Unsupported`]
//! until the docs collaborator stages the bitstream.

use crate::model::Thread;
use crate::{Design, Error, Result};

/// The HUS magic (u32 LE at offset 0).
pub const MAGIC: u32 = 0x00C8_AF5B;

/// A parsed HUS file: header metadata plus the raw compressed
/// streams.
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

/// Parses a HUS header and splits out the raw compressed streams.
pub fn parse(data: &[u8]) -> Result<HusFile> {
    if data.len() < 0x28 {
        return Err(Error::UnexpectedEof {
            context: "HUS header",
        });
    }
    let u32le = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let s16 = |o: usize| i16::from_le_bytes([data[o], data[o + 1]]);
    if u32le(0) != MAGIC {
        return Err(Error::BadMagic { expected: "HUS" });
    }
    let stitch_count = u32le(4);
    let colors = u32le(8) as usize;
    let extents = (s16(0x0C), s16(0x0E), s16(0x10), s16(0x12));
    let attr_off = u32le(0x14) as usize;
    let x_off = u32le(0x18) as usize;
    let y_off = u32le(0x1C) as usize;
    let string_offset = u32le(0x20);
    if colors > 0x100 {
        return Err(Error::invalid(format!("HUS colour count {colors}")));
    }
    if 0x28 + 2 * colors > data.len() {
        return Err(Error::UnexpectedEof {
            context: "HUS colour table",
        });
    }
    let palette: Vec<u16> = (0..colors)
        .map(|i| u16::from_le_bytes([data[0x28 + 2 * i], data[0x28 + 2 * i + 1]]))
        .collect();
    if !(attr_off <= x_off && x_off <= y_off && y_off <= data.len() && attr_off <= data.len()) {
        return Err(Error::invalid(
            "HUS stream offsets are not in ascending order inside the file",
        ));
    }
    Ok(HusFile {
        stitch_count,
        extents,
        palette,
        attributes: data[attr_off..x_off].to_vec(),
        x_deltas: data[x_off..y_off].to_vec(),
        y_deltas: data[y_off..].to_vec(),
        string_offset,
    })
}

/// Stitch decoding — blocked on the undocumented compression of the
/// three streams; always returns [`Error::Unsupported`].
pub fn decode_design(_file: &HusFile) -> Result<Design> {
    Err(Error::Unsupported {
        what: "HUS stitch streams use a compression scheme the staged documentation does not cover",
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

    #[test]
    fn parse_synthetic() {
        let bytes = synthetic();
        assert!(probe(&bytes));
        let f = parse(&bytes).unwrap();
        assert_eq!(f.stitch_count, 55184);
        assert_eq!(f.extents, (647, 907, -648, -908));
        assert_eq!(f.palette, vec![0, 26, 6]);
        assert_eq!(f.attributes, vec![1, 2, 3]);
        assert_eq!(f.x_deltas, vec![4, 5]);
        assert_eq!(f.y_deltas, vec![6, 7, 8, 9]);
        assert!(matches!(decode_design(&f), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn bad_magic_rejected() {
        assert!(matches!(parse(&[0u8; 64]), Err(Error::BadMagic { .. })));
    }
}
