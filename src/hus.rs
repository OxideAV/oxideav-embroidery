//! Husqvarna Viking HUS and VIP — decode + encode.
//!
//! Per the workspace's staged documentation (`docs/embroidery/hus/`,
//! header and stitch layer validated against 8 `.hus` and 3 `.vip`
//! real files): a little-endian header with the magic `0x00C8AF5B`,
//! stitch count, colour count, s16 extents, offsets to three
//! **separate compressed streams** (attributes, X deltas, Y deltas),
//! an inline 8-byte ASCII design label at `0x20`, and a u16 colour
//! table at `0x28`. Each stream is an independent ArchiveLib GL
//! bitstream ([`crate::gl`]) that decompresses to exactly
//! `stitch_count` bytes; stitch *i* is `(attr[i], x[i], y[i])` with
//! signed 8-bit deltas.
//!
//! Attribute bytes (all five values corpus-validated): `0x80` stitch,
//! `0x81` jump, `0x84` colour change, `0x88` trim, `0x90` end of
//! design (exactly one, always last).
//!
//! **VIP** shares the layout field for field with two differences:
//! its magic (`0x0190FC5D`), and an inserted record between the
//! colour table and the block area (replacing HUS's two filler
//! bytes): a 12-byte constant signature, three u32 fields equal to 1,
//! a u32 length in UTF-16 code units, and the design name in
//! UTF-16LE. The header's stream offsets already account for it; a
//! reader must take the offsets from the header. VIP's compressed
//! blocks are byte-identical to its `.hus` sibling's; its colour
//! table is on a different (undocumented) scale from HUS's.
//!
//! Unpinned encoder choices (documented, awaiting further staged
//! material): the two HUS filler bytes after the colour table (both
//! `0x00` and `0x1B 0x00` occur in the corpus; zeros are written
//! here), and the meaning of VIP's three constant u32 fields.

use crate::model::{Command, Thread};
use crate::{gl, Design, Error, Result};

/// The HUS magic (u32 LE at offset 0).
pub const MAGIC: u32 = 0x00C8_AF5B;

/// The VIP magic (u32 LE at offset 0).
pub const VIP_MAGIC: u32 = 0x0190_FC5D;

/// VIP's 12-byte inserted-record signature, identical across every
/// `.vip` in the validation corpus (corpus-observed constant).
pub const VIP_RECORD_SIGNATURE: [u8; 12] = [
    0x2E, 0x52, 0xB6, 0xD9, 0xE1, 0x54, 0x57, 0x91, 0xEA, 0x5C, 0x74, 0xD8,
];

/// Attribute byte: normal stitch.
pub const ATTR_STITCH: u8 = 0x80;
/// Attribute byte: jump (needle up).
pub const ATTR_JUMP: u8 = 0x81;
/// Attribute byte: colour change.
pub const ATTR_COLOR_CHANGE: u8 = 0x84;
/// Attribute byte: trim.
pub const ATTR_TRIM: u8 = 0x88;
/// Attribute byte: end of design.
pub const ATTR_END: u8 = 0x90;

/// Implementation cap on the declared stitch count (bounds the three
/// decompression buffers against hostile headers).
const MAX_STITCHES: u32 = 4_000_000;

/// A parsed HUS or VIP file: header metadata plus the raw compressed
/// streams (the two formats share this layout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HusFile {
    /// Declared stitch/record count (including the end record).
    pub stitch_count: u32,
    /// Extents: (+X, +Y, −X, −Y) as stored, 0.1 mm.
    pub extents: (i16, i16, i16, i16),
    /// Husqvarna thread indices, one per colour. A VIP's table is on
    /// a different (undocumented) scale from its HUS sibling's.
    pub palette: Vec<u16>,
    /// The inline 8-byte ASCII design label at 0x20 (HUS; VIP zeroes
    /// it). `None` when all-zero.
    pub label: Option<String>,
    /// The UTF-16LE design name from VIP's inserted record. `None`
    /// for HUS.
    pub vip_name: Option<String>,
    /// Raw compressed attribute (command) stream.
    pub attributes: Vec<u8>,
    /// Raw compressed X-delta stream.
    pub x_deltas: Vec<u8>,
    /// Raw compressed Y-delta stream.
    pub y_deltas: Vec<u8>,
    /// HUS: the two undocumented filler bytes after the colour table
    /// (`0x00 0x00` and `0x1B 0x00` both occur in the corpus).
    /// Empty for VIP (its inserted record is parsed instead).
    pub filler: Vec<u8>,
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

/// A decoded HUS/VIP file: the parsed container plus the stitch
/// design recovered from the three compressed streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HusDecoded {
    /// Parsed header/container state.
    pub file: HusFile,
    /// The stitch design.
    pub design: Design,
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
/// for VIP's inserted name record, which is parsed into
/// [`HusFile::vip_name`].
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
    // Inline 8-byte ASCII label, NUL-terminated when short, all-zero
    // when absent (corpus-corrected reading of 0x20).
    let label_bytes = &data[0x20..0x28];
    let label = match label_bytes.iter().position(|&b| b == 0) {
        Some(0) => None,
        end => {
            let s = &label_bytes[..end.unwrap_or(8)];
            Some(String::from_utf8_lossy(s).into_owned())
        }
    };
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
    let gap = &data[table_end..attr_off];
    let (vip_name, filler) = if magic == VIP_MAGIC {
        (Some(parse_vip_record(gap)?), Vec::new())
    } else {
        (None, gap.to_vec())
    };
    Ok(HusFile {
        stitch_count,
        extents,
        palette,
        label,
        vip_name,
        attributes: data[attr_off..x_off].to_vec(),
        x_deltas: data[x_off..y_off].to_vec(),
        y_deltas: data[y_off..].to_vec(),
        filler,
    })
}

/// Parses VIP's inserted record: 12-byte signature, three u32 fields
/// (1 in every corpus file, meaning undocumented), a u32 length in
/// UTF-16 code units including the terminator, and the UTF-16LE
/// design name.
fn parse_vip_record(gap: &[u8]) -> Result<String> {
    if gap.len() < 28 {
        return Err(Error::UnexpectedEof {
            context: "VIP inserted record",
        });
    }
    let len = u32::from_le_bytes([gap[24], gap[25], gap[26], gap[27]]) as usize;
    if 28 + 2 * len != gap.len() {
        return Err(Error::invalid(format!(
            "VIP name length {len} inconsistent with a {}-byte record area",
            gap.len()
        )));
    }
    let units: Vec<u16> = (0..len)
        .map(|i| u16::from_le_bytes([gap[28 + 2 * i], gap[28 + 2 * i + 1]]))
        .collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    Ok(String::from_utf16_lossy(&units[..end]))
}

/// Decompresses the three streams and rebuilds the stitch design.
pub fn decode_design(file: &HusFile) -> Result<Design> {
    if file.stitch_count > MAX_STITCHES {
        return Err(Error::invalid(format!(
            "declared stitch count {} exceeds the implementation cap",
            file.stitch_count
        )));
    }
    let n = file.stitch_count as usize;
    let stream = |raw: &[u8], what: &'static str| -> Result<Vec<u8>> {
        let (out, _) = gl::decompress(raw, n)?;
        if out.len() != n {
            return Err(Error::invalid(format!(
                "{what} stream decompresses to {} bytes for a {n}-record design",
                out.len()
            )));
        }
        Ok(out)
    };
    let attrs = stream(&file.attributes, "attribute")?;
    let xs = stream(&file.x_deltas, "X-delta")?;
    let ys = stream(&file.y_deltas, "Y-delta")?;

    let mut commands = Vec::with_capacity(n);
    for i in 0..n {
        let dx = xs[i] as i8 as i32;
        let dy = ys[i] as i8 as i32;
        match attrs[i] {
            ATTR_STITCH => commands.push(Command::Stitch { dx, dy }),
            ATTR_JUMP => commands.push(Command::Jump { dx, dy }),
            ATTR_COLOR_CHANGE => commands.push(Command::ColorChange {
                dx,
                dy,
                index: None,
            }),
            ATTR_TRIM => commands.push(Command::Trim { dx, dy }),
            ATTR_END => {
                commands.push(Command::End);
                if i + 1 != n {
                    return Err(Error::invalid(
                        "HUS end-of-design record is not the last record",
                    ));
                }
            }
            other => {
                return Err(Error::invalid(format!(
                    "HUS attribute byte 0x{other:02x} not documented"
                )));
            }
        }
    }
    Ok(Design {
        commands,
        threads: file.threads(),
        label: file
            .vip_name
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| file.label.clone()),
    })
}

/// Probes, parses and decodes a HUS or VIP file in one step.
pub fn decode(data: &[u8]) -> Result<HusDecoded> {
    let file = if probe(data) {
        parse(data)?
    } else if probe_vip(data) {
        parse_vip(data)?
    } else {
        return Err(Error::BadMagic {
            expected: "HUS/VIP",
        });
    };
    let design = decode_design(&file)?;
    Ok(HusDecoded { file, design })
}

// ───────────────────────── encoding ─────────────────────────

/// Encoding options shared by HUS and VIP.
#[derive(Debug, Clone, Default)]
pub struct HusEncodeOptions {
    /// Design label. HUS stores up to 8 ASCII characters inline at
    /// 0x20; VIP stores the full name UTF-16LE in its inserted record
    /// (and zeroes the inline field, per the corpus).
    pub label: String,
    /// Husqvarna thread indices, one per colour block. When empty,
    /// indices are taken from `design.threads` palette indices,
    /// falling back to `0, 1, 2, …`. Note the staged docs pin no
    /// index scale, and VIP's differs from HUS's.
    pub palette: Vec<u16>,
}

/// The three per-stitch byte streams of a design, pre-compression.
fn build_streams(design: &Design) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let mut attrs = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut push = |attr: u8, dx: i32, dy: i32| {
        attrs.push(attr);
        xs.push((dx as i8) as u8);
        ys.push((dy as i8) as u8);
    };
    // Splits an arbitrary move into ±127 chunks of the given kind.
    fn split(push: &mut impl FnMut(u8, i32, i32), attr: u8, mut dx: i32, mut dy: i32) {
        loop {
            let sx = dx.clamp(-127, 127);
            let sy = dy.clamp(-127, 127);
            push(attr, sx, sy);
            dx -= sx;
            dy -= sy;
            if dx == 0 && dy == 0 {
                return;
            }
        }
    }
    // Implementation cap (±10 m) bounding the splitting loop; the
    // header extents are s16 and enforce the real format limit below.
    let pre = design.extents();
    if pre.min_x < -1_000_000
        || pre.max_x > 1_000_000
        || pre.min_y < -1_000_000
        || pre.max_y > 1_000_000
    {
        return Err(Error::OutOfRange {
            field: "HUS extents (implementation cap)",
        });
    }
    for c in &design.commands {
        match *c {
            Command::Stitch { dx, dy } => split(&mut push, ATTR_STITCH, dx, dy),
            Command::Jump { dx, dy } => split(&mut push, ATTR_JUMP, dx, dy),
            // Colour changes and trims carry (0, 0) in every corpus
            // file; a non-zero model delta is expressed as jumps
            // first.
            Command::ColorChange { dx, dy, .. } => {
                if dx != 0 || dy != 0 {
                    split(&mut push, ATTR_JUMP, dx, dy);
                }
                push(ATTR_COLOR_CHANGE, 0, 0);
            }
            Command::Trim { dx, dy } => {
                if dx != 0 || dy != 0 {
                    split(&mut push, ATTR_JUMP, dx, dy);
                }
                push(ATTR_TRIM, 0, 0);
            }
            Command::Stop => {
                return Err(Error::Unsupported {
                    what: "HUS has no stop record distinct from a colour change",
                });
            }
            Command::End => break,
        }
    }
    push(ATTR_END, 0, 0);
    Ok((attrs, xs, ys))
}

fn palette_for(design: &Design, options: &HusEncodeOptions) -> Result<Vec<u16>> {
    let blocks = design.color_block_count().max(1);
    let palette: Vec<u16> = if !options.palette.is_empty() {
        options.palette.clone()
    } else if design.threads.len() >= blocks {
        design
            .threads
            .iter()
            .take(blocks)
            .enumerate()
            .map(|(i, t)| t.palette_index.unwrap_or(i as u16))
            .collect()
    } else {
        (0..blocks as u16).collect()
    };
    if palette.len() != blocks {
        return Err(Error::invalid(format!(
            "colour table has {} entries for {} colour blocks",
            palette.len(),
            blocks
        )));
    }
    Ok(palette)
}

fn encode_with(design: &Design, options: &HusEncodeOptions, vip: bool) -> Result<Vec<u8>> {
    let palette = palette_for(design, options)?;
    let (attrs, xs, ys) = build_streams(design)?;
    let e = design.extents();
    for v in [e.max_x, e.max_y, e.min_x, e.min_y] {
        if v < i16::MIN as i32 || v > i16::MAX as i32 {
            return Err(Error::OutOfRange {
                field: "HUS extents (header fields are s16)",
            });
        }
    }
    if !options.label.is_ascii() {
        return Err(Error::OutOfRange { field: "HUS label" });
    }
    if !vip && options.label.len() > 8 {
        return Err(Error::OutOfRange {
            field: "HUS label (8 ASCII characters)",
        });
    }
    let ca = gl::compress(&attrs);
    let cx = gl::compress(&xs);
    let cy = gl::compress(&ys);

    let mut inserted = Vec::new();
    if vip {
        inserted.extend_from_slice(&VIP_RECORD_SIGNATURE);
        for _ in 0..3 {
            inserted.extend_from_slice(&1u32.to_le_bytes());
        }
        let units: Vec<u16> = options.label.encode_utf16().chain([0]).collect();
        inserted.extend_from_slice(&(units.len() as u32).to_le_bytes());
        for u in units {
            inserted.extend_from_slice(&u.to_le_bytes());
        }
    } else {
        // The two filler bytes after the colour table are unpinned
        // (0x00 0x00 and 0x1B 0x00 both occur in the corpus).
        inserted.extend_from_slice(&[0x00, 0x00]);
    }

    let attr_off = 0x28 + 2 * palette.len() + inserted.len();
    let mut out = Vec::with_capacity(attr_off + ca.len() + cx.len() + cy.len());
    out.extend_from_slice(&if vip { VIP_MAGIC } else { MAGIC }.to_le_bytes());
    out.extend_from_slice(&(attrs.len() as u32).to_le_bytes());
    out.extend_from_slice(&(palette.len() as u32).to_le_bytes());
    for v in [e.max_x, e.max_y, e.min_x, e.min_y] {
        out.extend_from_slice(&(v as i16).to_le_bytes());
    }
    out.extend_from_slice(&(attr_off as u32).to_le_bytes());
    out.extend_from_slice(&((attr_off + ca.len()) as u32).to_le_bytes());
    out.extend_from_slice(&((attr_off + ca.len() + cx.len()) as u32).to_le_bytes());
    // Inline 8-byte ASCII label; VIP zeroes it (corpus finding).
    let mut label8 = [0u8; 8];
    if !vip {
        for (dst, src) in label8.iter_mut().zip(options.label.bytes()) {
            *dst = src;
        }
    }
    out.extend_from_slice(&label8);
    for p in &palette {
        out.extend_from_slice(&p.to_le_bytes());
    }
    out.extend_from_slice(&inserted);
    out.extend_from_slice(&ca);
    out.extend_from_slice(&cx);
    out.extend_from_slice(&cy);
    Ok(out)
}

/// Encodes a design as a HUS file.
pub fn encode(design: &Design, options: &HusEncodeOptions) -> Result<Vec<u8>> {
    encode_with(design, options, false)
}

/// Encodes a design as a VIP file. The label is stored UTF-16LE in
/// VIP's inserted record. Note the staged docs leave VIP's colour
/// index scale undocumented; the palette is written as given.
pub fn encode_vip(design: &Design, options: &HusEncodeOptions) -> Result<Vec<u8>> {
    encode_with(design, options, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Design {
        Design {
            commands: vec![
                Command::Jump { dx: 300, dy: -10 },
                Command::Stitch { dx: 5, dy: -5 },
                Command::Stitch { dx: -9, dy: 4 },
                Command::Trim { dx: 0, dy: 0 },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::Stitch { dx: 100, dy: 100 },
                Command::End,
            ],
            ..Default::default()
        }
    }

    #[test]
    fn hus_roundtrip() {
        let d = sample();
        let bytes = encode(
            &d,
            &HusEncodeOptions {
                label: "TESTLBL".into(),
                palette: vec![0, 26],
            },
        )
        .unwrap();
        assert!(probe(&bytes));
        assert!(!probe_vip(&bytes));
        let got = decode(&bytes).unwrap();
        assert_eq!(got.file.palette, vec![0, 26]);
        assert_eq!(got.file.label.as_deref(), Some("TESTLBL"));
        assert_eq!(got.file.vip_name, None);
        assert_eq!(got.file.filler, vec![0, 0]);
        // The oversized jump splits into three; everything else is
        // preserved verbatim (colour change / trim carry (0,0)).
        assert_eq!(got.design.counts().jumps, 3);
        assert_eq!(got.design.counts().stitches, 3);
        assert_eq!(got.design.counts().trims, 1);
        assert_eq!(got.design.counts().color_changes, 1);
        assert_eq!(got.design.positions().last(), d.positions().last());
        assert_eq!(got.design.label.as_deref(), Some("TESTLBL"));
        // Header bookkeeping: stitch count and extents.
        let f = parse(&bytes).unwrap();
        assert_eq!(f.stitch_count as usize, got.design.commands.len());
        let e = d.extents();
        assert_eq!(
            f.extents,
            (
                e.max_x as i16,
                e.max_y as i16,
                e.min_x as i16,
                e.min_y as i16
            )
        );
    }

    #[test]
    fn vip_roundtrip_with_long_name() {
        let d = sample();
        let bytes = encode_vip(
            &d,
            &HusEncodeOptions {
                label: "a design name longer than eight chars".into(),
                palette: vec![0, 70],
            },
        )
        .unwrap();
        assert!(probe_vip(&bytes));
        assert!(!probe(&bytes));
        let got = decode(&bytes).unwrap();
        assert_eq!(got.file.label, None); // inline field zeroed
        assert_eq!(
            got.file.vip_name.as_deref(),
            Some("a design name longer than eight chars")
        );
        assert_eq!(got.file.palette, vec![0, 70]);
        assert_eq!(got.design.counts(), decode(&bytes).unwrap().design.counts());
    }

    #[test]
    fn hus_and_vip_streams_are_byte_identical() {
        // The corpus finding: a VIP and its HUS sibling carry
        // byte-identical compressed blocks.
        let d = sample();
        let opts = HusEncodeOptions {
            label: "twin".into(),
            palette: vec![1, 2],
        };
        let h = parse(&encode(&d, &opts).unwrap()).unwrap();
        let v = parse_vip(&encode_vip(&d, &opts).unwrap()).unwrap();
        assert_eq!(h.attributes, v.attributes);
        assert_eq!(h.x_deltas, v.x_deltas);
        assert_eq!(h.y_deltas, v.y_deltas);
        assert_eq!(h.stitch_count, v.stitch_count);
        assert_eq!(h.extents, v.extents);
    }

    #[test]
    fn label_too_long_for_hus_rejected() {
        let d = sample();
        let r = encode(
            &d,
            &HusEncodeOptions {
                label: "nine char".into(),
                ..Default::default()
            },
        );
        assert!(matches!(r, Err(Error::OutOfRange { .. })));
    }

    #[test]
    fn stop_rejected() {
        let d = Design {
            commands: vec![Command::Stop, Command::End],
            ..Default::default()
        };
        assert!(matches!(
            encode(&d, &HusEncodeOptions::default()),
            Err(Error::Unsupported { .. })
        ));
    }

    #[test]
    fn match_compressed_streams_decode_identically() {
        // A hypothetical producer using the documented GL match
        // layer: re-compress the three streams with the match-bearing
        // encoder — the container decoder must recover the identical
        // design from them.
        let base = sample();
        let mut commands = Vec::new();
        for _ in 0..40 {
            commands.extend_from_slice(&base.commands[..base.commands.len() - 1]);
        }
        commands.push(Command::End);
        let d = Design {
            commands,
            ..Default::default()
        };
        let bytes = encode(&d, &HusEncodeOptions::default()).unwrap();
        let plain = decode(&bytes).unwrap();
        let mut f = plain.file.clone();
        let n = f.stitch_count as usize;
        let (attrs, _) = gl::decompress(&f.attributes, n).unwrap();
        let (xs, _) = gl::decompress(&f.x_deltas, n).unwrap();
        let (ys, _) = gl::decompress(&f.y_deltas, n).unwrap();
        f.attributes = gl::compress_lz(&attrs, 16384).unwrap();
        f.x_deltas = gl::compress_lz(&xs, 16384).unwrap();
        f.y_deltas = gl::compress_lz(&ys, 16384).unwrap();
        // The repeats guarantee the match layer actually fired.
        assert!(f.attributes.len() < plain.file.attributes.len());
        assert_eq!(decode_design(&f).unwrap(), plain.design);
    }

    #[test]
    fn undocumented_attribute_rejected() {
        // Rebuild a valid file with one attribute byte corrupted.
        let d = sample();
        let bytes = encode(&d, &HusEncodeOptions::default()).unwrap();
        let mut f = parse(&bytes).unwrap();
        let n = f.stitch_count as usize;
        let (mut attrs, _) = gl::decompress(&f.attributes, n).unwrap();
        attrs[0] = 0x82; // never observed in the corpus
        f.attributes = gl::compress(&attrs);
        assert!(matches!(decode_design(&f), Err(Error::Invalid { .. })));
    }

    #[test]
    fn end_record_must_be_last() {
        let d = sample();
        let bytes = encode(&d, &HusEncodeOptions::default()).unwrap();
        let mut f = parse(&bytes).unwrap();
        let n = f.stitch_count as usize;
        let (mut attrs, _) = gl::decompress(&f.attributes, n).unwrap();
        attrs[0] = ATTR_END;
        f.attributes = gl::compress(&attrs);
        assert!(matches!(decode_design(&f), Err(Error::Invalid { .. })));
    }

    #[test]
    fn stitch_count_mismatch_rejected() {
        let d = sample();
        let mut bytes = encode(&d, &HusEncodeOptions::default()).unwrap();
        let n = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        bytes[4..8].copy_from_slice(&(n + 1).to_le_bytes());
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn hostile_stitch_count_capped() {
        let d = sample();
        let mut bytes = encode(&d, &HusEncodeOptions::default()).unwrap();
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(decode(&bytes), Err(Error::Invalid { .. })));
    }

    #[test]
    fn bad_magic_rejected() {
        assert!(matches!(parse(&[0u8; 64]), Err(Error::BadMagic { .. })));
        assert!(matches!(parse_vip(&[0u8; 64]), Err(Error::BadMagic { .. })));
        assert!(matches!(decode(&[0u8; 64]), Err(Error::BadMagic { .. })));
    }

    #[test]
    fn stream_offset_inside_color_table_rejected() {
        let d = sample();
        let mut bytes = encode(&d, &HusEncodeOptions::default()).unwrap();
        bytes[0x14..0x18].copy_from_slice(&0x10u32.to_le_bytes());
        assert!(matches!(parse(&bytes), Err(Error::Invalid { .. })));
    }
}
