//! Brother PHX — current-generation machine-side design format.
//! Best-effort decode; **unvalidated**.
//!
//! The workspace's staged documentation
//! (`docs/embroidery/phx/phx-observed-structure.md`) derives PHX
//! entirely from the format's own vendor reader; **no real `.phx`
//! sample exists anywhere in the staged corpus**, so unlike every
//! other module in this crate this parser has never been checked
//! against a real file. It follows the documented reader walk:
//!
//! - `#PHX` + 4-byte ASCII version (`"0200"`; other values are
//!   tolerated, as in the vendor reader),
//! - a u32 at 0x08 and a 36-byte skipped region (0x0C..0x2F, the
//!   copyright-string position by analogy with PHC),
//! - a fixed header at 0x30 carrying stored file offsets (all
//!   little-endian) and four s16 geometry fields,
//! - a colour list at 0x4C: s16 count, then 8 bytes per colour
//!   including a **real RGB triple**,
//! - a bitmap section at a stored offset: two skipped u32s, s16
//!   width/height, then `(width × height × colours + 7) >> 3` bytes
//!   of 1-bit images,
//! - a chunked body (`#EMB` / `#GRP` / `#PTN` / `#VAR`, 4-byte ASCII
//!   tags then 32-bit fields); `#VAR` holds a u32 offset to the PEC
//!   stitch data.
//!
//! Two points the staged material leaves open are handled
//! defensively: the chunk length convention (the first u32 after
//! each tag is treated as the payload length) and the base the
//! `#VAR` offset is relative to (several documented-plausible bases
//! are tried and the first that yields a well-formed PEC stream
//! wins).

use crate::model::{Design, Thread};
use crate::pec;
use crate::{Error, Result};

/// One PHX colour record (8 bytes on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhxColor {
    /// The two leading record bytes.
    pub pair: (u8, u8),
    /// The s16 field at record offset 2.
    pub field: i16,
    /// The RGB triple.
    pub rgb: [u8; 3],
    /// The trailing per-colour byte (indexed during stitch decode by
    /// the vendor reader).
    pub tag: u8,
}

/// A decoded PHX file (see the module caveat: unvalidated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhxFile {
    /// ASCII version string (expected `"0200"`).
    pub version: String,
    /// The 36-byte skipped region at 0x0C, trimmed as text.
    pub copyright: String,
    /// Header u32 fields at 0x34/0x38/0x3C/0x40 (offsets A and B,
    /// a design field, and the absolute section offset C).
    pub header_fields: [u32; 4],
    /// The four s16 geometry fields at 0x44.
    pub geometry: [i16; 4],
    /// Colour records, RGB included.
    pub colors: Vec<PhxColor>,
    /// Bitmap section width/height in pixels.
    pub bitmap_width: i16,
    pub bitmap_height: i16,
    /// The raw bitmap section (`(w × h × colours + 7) >> 3` bytes;
    /// per-colour images are bit-packed and not byte-aligned).
    pub bitmaps: Vec<u8>,
    /// Chunk tags found in the body, in file order.
    pub chunks: Vec<[u8; 4]>,
    /// Absolute file offset at which the PEC stitch stream was found.
    pub stitch_offset: usize,
    /// The stitch design decoded from the PEC stream.
    pub design: Design,
}

/// Returns true when `data` starts with the `#PHX` signature.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 8 && &data[..4] == b"#PHX"
}

fn find_chunk(data: &[u8], tag: &[u8; 4], from: usize) -> Option<usize> {
    data.get(from..)?
        .windows(4)
        .position(|w| w == tag)
        .map(|p| p + from)
}

/// Decodes a PHX file.
pub fn decode(data: &[u8]) -> Result<PhxFile> {
    if data.len() < 4 || &data[..4] != b"#PHX" {
        return Err(Error::BadMagic { expected: "PHX" });
    }
    if data.len() < 0x4E {
        return Err(Error::UnexpectedEof {
            context: "PHX header",
        });
    }
    let version = String::from_utf8_lossy(&data[4..8]).into_owned();
    let copyright = String::from_utf8_lossy(&data[0x0C..0x30])
        .trim_end_matches([' ', '\0'])
        .to_string();
    let u32le = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let s16 = |o: usize| i16::from_le_bytes([data[o], data[o + 1]]);
    let header_fields = [u32le(0x34), u32le(0x38), u32le(0x3C), u32le(0x40)];
    let geometry = [s16(0x44), s16(0x46), s16(0x48), s16(0x4A)];

    // Colour list at 0x4C.
    let count = s16(0x4C);
    if !(0..=256).contains(&count) {
        return Err(Error::invalid(format!("PHX colour count {count}")));
    }
    let count = count as usize;
    let list_end = 0x4E + count * 8;
    if list_end > data.len() {
        return Err(Error::UnexpectedEof {
            context: "PHX colour list",
        });
    }
    let mut colors = Vec::with_capacity(count);
    for i in 0..count {
        let r = &data[0x4E + i * 8..0x4E + i * 8 + 8];
        colors.push(PhxColor {
            pair: (r[0], r[1]),
            field: i16::from_le_bytes([r[2], r[3]]),
            rgb: [r[4], r[5], r[6]],
            tag: r[7],
        });
    }

    // Bitmap section at the stored absolute offset (header field C).
    let bitmap_offset = header_fields[3] as usize;
    let (bitmap_width, bitmap_height, bitmaps, bitmap_end) =
        if bitmap_offset >= list_end && bitmap_offset + 12 <= data.len() {
            let w = i16::from_le_bytes([data[bitmap_offset + 8], data[bitmap_offset + 9]]);
            let h = i16::from_le_bytes([data[bitmap_offset + 10], data[bitmap_offset + 11]]);
            let len = ((w.max(0) as usize) * (h.max(0) as usize) * count + 7) >> 3;
            let start = bitmap_offset + 12;
            if start + len <= data.len() {
                (w, h, data[start..start + len].to_vec(), start + len)
            } else {
                (w, h, Vec::new(), list_end)
            }
        } else {
            (0, 0, Vec::new(), list_end)
        };

    // Chunked body: record the tags present, locate #VAR.
    let mut chunks = Vec::new();
    for tag in [b"#EMB", b"#GRP", b"#PTN", b"#VAR"] {
        if find_chunk(data, tag, list_end.min(data.len())).is_some() {
            chunks.push(*tag);
        }
    }
    let var_pos = find_chunk(data, b"#VAR", list_end.min(data.len()))
        .ok_or_else(|| Error::invalid("PHX has no #VAR chunk (stitch data locator)"))?;
    if var_pos + 8 > data.len() {
        return Err(Error::UnexpectedEof {
            context: "PHX #VAR chunk",
        });
    }
    // The #VAR payload leads with a u32 offset to the stitch data.
    // Depending on the chunk length convention the offset field is
    // either directly after the tag or after a leading length u32;
    // the base it is relative to is not pinned by the staged
    // documentation. Try the documented-plausible readings.
    let mut candidates = Vec::new();
    for field_pos in [var_pos + 4, var_pos + 8] {
        if field_pos + 4 > data.len() {
            continue;
        }
        let v = u32::from_le_bytes(data[field_pos..field_pos + 4].try_into().unwrap()) as usize;
        for base in [0usize, field_pos + 4, bitmap_end, list_end] {
            candidates.push(base.saturating_add(v));
        }
    }
    // Try the most specific reading first: among candidates that
    // decode as a well-formed PEC stream, prefer one whose stream
    // ends exactly at EOF, and among those the latest start (an
    // earlier false start would decode a garbage prefix into
    // spurious stitches before reaching the same terminator).
    candidates.sort_unstable_by(|a, b| b.cmp(a));
    candidates.dedup();
    let mut found = None;
    let mut fallback = None;
    for cand in candidates {
        if cand >= data.len() {
            continue;
        }
        if let Ok((commands, _leading, used)) = pec::decode_stitches(&data[cand..]) {
            if cand + used == data.len() {
                found = Some((cand, commands));
                break;
            }
            if fallback.is_none() {
                fallback = Some((cand, commands));
            }
        }
    }
    let found = found.or(fallback);
    let (stitch_offset, commands) = found.ok_or_else(|| {
        Error::invalid("PHX #VAR offset does not lead to a well-formed PEC stream")
    })?;

    let threads = colors
        .iter()
        .map(|c| Thread {
            palette_index: None,
            rgb: Some(c.rgb),
            catalog: None,
            name: None,
        })
        .collect();

    Ok(PhxFile {
        version,
        copyright,
        header_fields,
        geometry,
        colors,
        bitmap_width,
        bitmap_height,
        bitmaps,
        chunks,
        stitch_offset,
        design: Design {
            commands,
            threads,
            label: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Design};

    /// Builds a synthetic PHX file following the documented reader
    /// walk. Self-consistency only: no real `.phx` exists to compare
    /// against, so this exercises the parser's reading of the staged
    /// structure, not the format itself.
    fn synthesize(colors: &[[u8; 3]], design: &Design) -> Vec<u8> {
        let n = colors.len();
        let stitches = crate::pec::encode_stitches(design).unwrap();
        let list_end = 0x4E + n * 8;
        let bitmap_offset = list_end;
        let (w, h) = (48i16, 38i16);
        let bitmap_len = ((w as usize) * (h as usize) * n + 7) >> 3;
        let chunks_start = bitmap_offset + 12 + bitmap_len;
        // #EMB (8-byte payload), #GRP (12), #PTN (8), then #VAR.
        let var_pos = chunks_start + (8 + 8) + (8 + 12) + (8 + 8);
        let stitch_abs = var_pos + 12;
        let mut out = Vec::new();
        out.extend_from_slice(b"#PHX");
        out.extend_from_slice(b"0200");
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(format!("{:<36}", "Synthetic fixture, this crate").as_bytes());
        assert_eq!(out.len(), 0x30);
        out.extend_from_slice(&0u32.to_le_bytes()); // 0x30
        out.extend_from_slice(&(chunks_start as u32).to_le_bytes()); // A
        out.extend_from_slice(&0u32.to_le_bytes()); // B
        out.extend_from_slice(&0u32.to_le_bytes()); // design field
        out.extend_from_slice(&(bitmap_offset as u32).to_le_bytes()); // C
        let e = design.extents();
        for v in [
            e.min_x as i16,
            e.max_x as i16,
            e.min_y as i16,
            e.max_y as i16,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(out.len(), 0x4C);
        out.extend_from_slice(&(n as i16).to_le_bytes());
        for (i, rgb) in colors.iter().enumerate() {
            out.push(0);
            out.push(0);
            out.extend_from_slice(&0i16.to_le_bytes());
            out.extend_from_slice(rgb);
            out.push(i as u8);
        }
        assert_eq!(out.len(), list_end);
        // Bitmap section: two skipped u32s, w, h, bit-packed images.
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&vec![0u8; bitmap_len]);
        assert_eq!(out.len(), chunks_start);
        out.extend_from_slice(b"#EMB");
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 8]);
        out.extend_from_slice(b"#GRP");
        out.extend_from_slice(&12u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(b"#PTN");
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&[0u8; 8]);
        assert_eq!(out.len(), var_pos);
        out.extend_from_slice(b"#VAR");
        out.extend_from_slice(&((4 + stitches.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(stitch_abs as u32).to_le_bytes());
        assert_eq!(out.len(), stitch_abs);
        out.extend_from_slice(&stitches);
        out
    }

    fn sample() -> Design {
        Design {
            commands: vec![
                Command::Jump { dx: 30, dy: 30 },
                Command::Stitch { dx: 8, dy: 0 },
                Command::Stitch { dx: 0, dy: 8 },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::Stitch { dx: -8, dy: -8 },
                Command::End,
            ],
            ..Default::default()
        }
    }

    #[test]
    fn decode_synthetic() {
        let d = sample();
        let bytes = synthesize(&[[255, 0, 0], [0, 0, 255]], &d);
        assert!(probe(&bytes));
        let f = decode(&bytes).unwrap();
        assert_eq!(f.version, "0200");
        assert_eq!(f.copyright, "Synthetic fixture, this crate");
        assert_eq!(f.colors.len(), 2);
        assert_eq!(f.colors[0].rgb, [255, 0, 0]);
        assert_eq!(f.design.threads[1].rgb, Some([0, 0, 255]));
        assert_eq!(f.bitmap_width, 48);
        assert_eq!(f.bitmap_height, 38);
        assert_eq!(f.chunks.len(), 4);
        assert_eq!(f.design.counts(), d.counts());
    }

    #[test]
    fn missing_var_rejected() {
        let d = sample();
        let mut bytes = synthesize(&[[1, 2, 3]], &d);
        let pos = bytes.windows(4).position(|w| w == b"#VAR").unwrap();
        bytes[pos..pos + 4].copy_from_slice(b"#XXX");
        assert!(matches!(decode(&bytes), Err(Error::Invalid { .. })));
    }

    #[test]
    fn bad_magic_rejected() {
        assert!(matches!(decode(&[0u8; 200]), Err(Error::BadMagic { .. })));
    }
}
