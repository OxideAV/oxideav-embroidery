//! Husqvarna Viking HUS and VIP — decode + encode.
//!
//! Per the workspace's staged documentation (`docs/embroidery/hus/`,
//! header and stitch layer validated against 8 `.hus` and 3 `.vip`
//! real files, then against 33 redistributable match-coded fixtures):
//! a little-endian header with one of the two documented HUS magics
//! (`0x00C8AF5B` and `0x00C8FC5D` — the high half `0x00C8` is the
//! stable part; neither value implies a literal-only writer), stitch
//! count, colour count, s16 extents, offsets to three
//! **separate compressed streams** (attributes, X deltas, Y deltas),
//! an inline 8-byte ASCII design label at `0x20`, and a u16 colour
//! table at `0x28`. Each stream is an independent ArchiveLib GL
//! bitstream ([`crate::gl`]) that decompresses to exactly
//! `stitch_count` bytes and ends on the end-of-stream code; a block's
//! header-delimited span may carry one unused byte after the byte
//! holding that code, which the reader ignores. Stitch *i* is `(attr[i], x[i], y[i])` with
//! signed 8-bit deltas.
//!
//! Attribute bytes (all five values corpus-validated): `0x80` stitch,
//! `0x81` jump, `0x84` colour change, `0x88` trim, `0x90` end of
//! design (at most one, always last). One producer of the staged
//! fixture set writes HUS streams with **no** end-of-design record
//! (its VIP siblings carry one); the decoder accepts that and the
//! design then simply has no [`Command::End`]. The encoder always
//! closes the streams with the end record.
//!
//! **VIP** shares the layout field for field with two differences:
//! its magic (`0x0190FC5D`), and an inserted record between the
//! colour table and the block area (replacing HUS's two filler
//! bytes). In the validation corpus the record is a 12-byte constant
//! signature, three u32 fields equal to 1, a u32 length in UTF-16 code
//! units, and the design name in UTF-16LE; other producers write a
//! longer head before the same length-prefixed name. The reader keeps
//! the whole record raw ([`HusFile::vip_record`]) and takes the name
//! from the record's tail (the u32 length directly followed by that
//! many UTF-16LE units, the last one NUL). The header's stream offsets
//! already account for the record; a reader must take the offsets
//! from the header. VIP's compressed
//! blocks are byte-identical to its `.hus` sibling's; its colour
//! table is on a different (undocumented) scale from HUS's.
//!
//! Unpinned encoder choices (documented, awaiting further staged
//! material): the two HUS filler bytes after the colour table (both
//! `0x00` and `0x1B 0x00` occur in the corpus; zeros are written
//! here), and the meaning of VIP's three constant u32 fields.

use crate::model::{Command, Thread};
use crate::{Design, Error, Result, gl};

/// The HUS magic (u32 LE at offset 0) the encoder writes by default.
pub const MAGIC: u32 = 0x00C8_AF5B;

/// The second documented HUS magic, written by several producers of
/// match-coded streams.
pub const MAGIC_FC5D: u32 = 0x00C8_FC5D;

/// Every documented HUS magic.
pub const HUS_MAGICS: [u32; 2] = [MAGIC, MAGIC_FC5D];

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
#[non_exhaustive]
pub struct HusFile {
    /// The magic the file carries (one of [`HUS_MAGICS`], or
    /// [`VIP_MAGIC`]).
    pub magic: u32,
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
    /// for HUS, and for a VIP record whose tail is not a
    /// length-prefixed NUL-terminated name.
    pub vip_name: Option<String>,
    /// VIP: the raw inserted record between the colour table and the
    /// first stream (its head is only partly documented). Empty for
    /// HUS.
    pub vip_record: Vec<u8>,
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
#[non_exhaustive]
pub struct HusDecoded {
    /// Parsed header/container state.
    pub file: HusFile,
    /// The stitch design.
    pub design: Design,
}

/// Returns true when `data` starts with a documented HUS magic.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 4
        && HUS_MAGICS.contains(&u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

/// Returns true when `data` starts with the VIP magic.
pub fn probe_vip(data: &[u8]) -> bool {
    data.len() >= 4 && u32::from_le_bytes([data[0], data[1], data[2], data[3]]) == VIP_MAGIC
}

/// Parses a HUS header and splits out the raw compressed streams.
pub fn parse(data: &[u8]) -> Result<HusFile> {
    parse_with_magic(data, &HUS_MAGICS, "HUS")
}

/// Parses a VIP header and splits out the raw compressed streams.
/// The layout is HUS's; the stored stream offsets already account
/// for VIP's inserted name record, which is parsed into
/// [`HusFile::vip_name`].
pub fn parse_vip(data: &[u8]) -> Result<HusFile> {
    parse_with_magic(data, &[VIP_MAGIC], "VIP")
}

fn parse_with_magic(data: &[u8], magics: &[u32], name: &'static str) -> Result<HusFile> {
    if data.len() < 0x28 {
        return Err(Error::UnexpectedEof { context: name });
    }
    let u32le = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let s16 = |o: usize| i16::from_le_bytes([data[o], data[o + 1]]);
    let magic = u32le(0);
    if !magics.contains(&magic) {
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
    let (vip_name, vip_record, filler) = if magic == VIP_MAGIC {
        (vip_record_name(gap), gap.to_vec(), Vec::new())
    } else {
        (None, Vec::new(), gap.to_vec())
    };
    Ok(HusFile {
        magic,
        stitch_count,
        extents,
        palette,
        label,
        vip_name,
        vip_record,
        attributes: data[attr_off..x_off].to_vec(),
        x_deltas: data[x_off..y_off].to_vec(),
        y_deltas: data[y_off..].to_vec(),
        filler,
    })
}

/// Reads the design name from VIP's inserted record. The record ends
/// with a u32 length in UTF-16 code units (terminator included)
/// followed by that many UTF-16LE units, the last one NUL. The
/// corpus layout puts the length at record offset 24; when that does
/// not fit, the shortest self-consistent length-prefixed tail is
/// taken. `None` when no tail is consistent.
fn vip_record_name(gap: &[u8]) -> Option<String> {
    let u32_at = |o: usize| u32::from_le_bytes([gap[o], gap[o + 1], gap[o + 2], gap[o + 3]]);
    let name_at = |start: usize, len: usize| -> Option<String> {
        let units: Vec<u16> = (0..len)
            .map(|i| u16::from_le_bytes([gap[start + 2 * i], gap[start + 2 * i + 1]]))
            .collect();
        let (last, body) = units.split_last()?;
        (*last == 0 && !body.contains(&0)).then(|| String::from_utf16_lossy(body))
    };
    // The corpus layout: 12-byte signature, three u32, then the length.
    if gap.len() >= 28 {
        let len = u32_at(24) as usize;
        if len >= 1
            && 28 + 2 * len == gap.len()
            && let Some(n) = name_at(28, len)
        {
            return Some(n);
        }
    }
    // Any other head: the length-prefixed name is the record's tail.
    (1..)
        .take_while(|&len| 4 + 2 * len <= gap.len())
        .find_map(|len| {
            let off = gap.len() - 2 * len - 4;
            (u32_at(off) as usize == len)
                .then(|| name_at(off + 4, len))
                .flatten()
        })
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
#[non_exhaustive]
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
    /// HUS magic to write (one of [`HUS_MAGICS`]); `None` writes
    /// [`MAGIC`]. Ignored by [`encode_vip`], whose magic is fixed.
    pub magic: Option<u32>,
}

impl HusEncodeOptions {
    /// Set `label`.
    pub fn with_label(mut self, label: String) -> Self {
        self.label = label;
        self
    }

    /// Set `palette`.
    pub fn with_palette(mut self, palette: Vec<u16>) -> Self {
        self.palette = palette;
        self
    }

    /// Set `magic`.
    pub fn with_magic(mut self, magic: Option<u32>) -> Self {
        self.magic = magic;
        self
    }
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
    let magic = if vip {
        VIP_MAGIC
    } else {
        let m = options.magic.unwrap_or(MAGIC);
        if !HUS_MAGICS.contains(&m) {
            return Err(Error::OutOfRange {
                field: "HUS magic (must be a documented value)",
            });
        }
        m
    };
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
    out.extend_from_slice(&magic.to_le_bytes());
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
                magic: None,
            },
        )
        .unwrap();
        assert!(probe(&bytes));
        assert!(!probe_vip(&bytes));
        let got = decode(&bytes).unwrap();
        assert_eq!(got.file.palette, vec![0, 26]);
        assert_eq!(got.file.label.as_deref(), Some("TESTLBL"));
        assert_eq!(got.file.vip_name, None);
        assert!(got.file.vip_record.is_empty());
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
                magic: None,
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
            magic: None,
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
    fn second_documented_magic_roundtrips() {
        let d = sample();
        let opts = HusEncodeOptions::default().with_magic(Some(MAGIC_FC5D));
        let bytes = encode(&d, &opts).unwrap();
        assert_eq!(&bytes[..4], &[0x5D, 0xFC, 0xC8, 0x00]);
        assert!(probe(&bytes));
        assert!(!probe_vip(&bytes));
        let got = decode(&bytes).unwrap();
        assert_eq!(got.file.magic, MAGIC_FC5D);
        let plain = decode(&encode(&d, &HusEncodeOptions::default()).unwrap()).unwrap();
        assert_eq!(plain.file.magic, MAGIC);
        assert_eq!(got.design, plain.design);
        // Only documented values are writable.
        let bad = HusEncodeOptions::default().with_magic(Some(0x00C8_0000));
        assert!(matches!(encode(&d, &bad), Err(Error::OutOfRange { .. })));
        // VIP's magic is fixed whatever the option says.
        let v = encode_vip(&d, &opts).unwrap();
        assert!(probe_vip(&v));
    }

    #[test]
    fn one_unused_byte_after_end_of_stream_tolerated() {
        // A block's span may extend one byte past the byte holding the
        // end-of-stream code; the reader stops on the code.
        let d = sample();
        let bytes = encode(&d, &HusEncodeOptions::default()).unwrap();
        let mut f = parse(&bytes).unwrap();
        for pad in [0x00u8, 0x80, 0xC0] {
            let mut g = f.clone();
            g.attributes.push(pad);
            g.x_deltas.push(pad);
            g.y_deltas.push(pad);
            assert_eq!(decode_design(&g).unwrap(), decode_design(&f).unwrap());
        }
        f.attributes.clear();
        assert!(decode_design(&f).is_err());
    }

    #[test]
    fn vip_name_read_from_record_tail_under_a_longer_head() {
        // A producer writing a longer head (here 6 + 2 × 10 opaque
        // bytes and ten u32 fields) before the same length-prefixed
        // name: the name is still found at the record's tail.
        let mut rec = vec![0xA5u8; 26];
        for _ in 0..10 {
            rec.extend_from_slice(&1u32.to_le_bytes());
        }
        let name: Vec<u16> = "random1-wilcom".encode_utf16().chain([0]).collect();
        rec.extend_from_slice(&(name.len() as u32).to_le_bytes());
        for u in &name {
            rec.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(vip_record_name(&rec).as_deref(), Some("random1-wilcom"));
        // An empty name (length 1, just the terminator).
        let mut empty = vec![0x5Au8; 26];
        for _ in 0..10 {
            empty.extend_from_slice(&1u32.to_le_bytes());
        }
        empty.extend_from_slice(&1u32.to_le_bytes());
        empty.extend_from_slice(&[0, 0]);
        assert_eq!(vip_record_name(&empty).as_deref(), Some(""));
        // No consistent tail → no name (and no error).
        assert_eq!(vip_record_name(&[0xFF; 40]), None);
        assert_eq!(vip_record_name(&[]), None);
    }

    #[test]
    fn hus_without_end_record_decodes() {
        // Re-pack a valid file's streams without the closing 0x90.
        let d = sample();
        let bytes = encode(&d, &HusEncodeOptions::default()).unwrap();
        let mut f = parse(&bytes).unwrap();
        let n = f.stitch_count as usize;
        let cut = |raw: &mut Vec<u8>| {
            let (mut plain, _) = gl::decompress(raw, n).unwrap();
            plain.pop();
            *raw = gl::compress(&plain);
        };
        cut(&mut f.attributes);
        cut(&mut f.x_deltas);
        cut(&mut f.y_deltas);
        f.stitch_count -= 1;
        let got = decode_design(&f).unwrap();
        let full = decode_design(&parse(&bytes).unwrap()).unwrap();
        assert_eq!(got.commands[..], full.commands[..full.commands.len() - 1]);
        assert!(!matches!(got.commands.last(), Some(Command::End)));
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
