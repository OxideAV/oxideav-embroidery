//! Brother PEC — the machine-side stitch block, standalone (`.pec`)
//! and embedded (in `.pes`, `.phc`, `.phx`).
//!
//! Layout per the workspace's staged documentation
//! (`docs/embroidery/pes/`, validated against real files per
//! `docs/embroidery/provenance/05-corpus-validation.md`):
//!
//! - **Section 1** (512 bytes): `LA:` + 16-byte label + CR at 19,
//!   colour-count byte at 48 (stored as `colours − 1`), then one
//!   Brother thread-table index per colour.
//! - **Section 2** (20 bytes): a 3-byte LE pointer at +2 from the
//!   section start to the thumbnail area (measured from the section-2
//!   start), design width/height (extent span + 1) at +8/+10.
//! - **Stitch list** from block offset 532: per-axis short form
//!   (1 byte, 7-bit two's complement) or long form (2 bytes, top bit
//!   set, 3 control bits selecting jump/trim, 12-bit two's
//!   complement); colour change `FE B0` + an alternating 2/1 index
//!   byte; end `FF`. The stream opens with a `(0, 0)` pair that
//!   machines ignore.
//! - **Thumbnails**: 1-bit images, 48×38 (6-byte stride, 228 bytes),
//!   one master plus one per colour, MSB-first from top-left.
//!
//! Unpinned by the staged documentation and chosen here (auditable
//! encoder decisions): the fill byte for the unknown regions of
//! section 1 (space, `0x20`), the two u16-sized fields at section-2
//! offsets +0 and the 4 trailing bytes at +16 (zero), and the
//! constant pair `(480, 432)` at +12 observed across the validation
//! corpus.

use crate::model::{Command, Design, Thread};
use crate::{Error, Result};

/// Standalone `.pec` signature.
pub const MAGIC: &[u8; 8] = b"#PEC0001";
/// Byte length of section 1.
pub const SECTION1_LEN: usize = 512;
/// Byte length of section 2.
pub const SECTION2_LEN: usize = 20;
/// Stitch data offset from the start of a PEC block.
pub const STITCH_OFFSET: usize = SECTION1_LEN + SECTION2_LEN;
/// Thumbnail geometry: width, height, row stride in bytes.
pub const THUMB_WIDTH: usize = 48;
pub const THUMB_HEIGHT: usize = 38;
pub const THUMB_STRIDE: usize = 6;
/// Bytes per thumbnail image.
pub const THUMB_LEN: usize = THUMB_STRIDE * THUMB_HEIGHT;
/// Long-form per-axis delta range.
pub const MAX_DELTA: i32 = 2047;
pub const MIN_DELTA: i32 = -2048;

/// One entry of the Brother 64-colour thread table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadColor {
    /// 1-based table index as used on the wire.
    pub index: u8,
    /// Vendor catalogue code.
    pub code: &'static str,
    /// Colour name.
    pub name: &'static str,
    /// RGB value.
    pub rgb: [u8; 3],
}

macro_rules! tc {
    ($i:expr, $c:expr, $n:expr, $hex:expr) => {
        ThreadColor {
            index: $i,
            code: $c,
            name: $n,
            rgb: [
                (($hex >> 16) & 0xFF) as u8,
                (($hex >> 8) & 0xFF) as u8,
                ($hex & 0xFF) as u8,
            ],
        }
    };
}

/// The Brother 64-entry thread table (index, catalogue code, name,
/// RGB). Numeric/factual data transcribed from the workspace-staged
/// table in `docs/embroidery/pes/edutechwiki-embroidery-format-pec.html`
/// (data table extraction; verified upstream against machine
/// hardware).
pub const BROTHER_THREADS: [ThreadColor; 64] = [
    tc!(1, "007", "Prussian Blue", 0x1a0a94),
    tc!(2, "000", "Blue", 0x0f75ff),
    tc!(3, "534", "Teal Green", 0x00934c),
    tc!(4, "070", "Corn Flower Blue", 0xbabdfe),
    tc!(5, "800", "Red", 0xec0000),
    tc!(6, "000", "Reddish Brown", 0xe4995a),
    tc!(7, "620", "Magenta", 0xcc48ab),
    tc!(8, "810", "Light Lilac", 0xfdc4fa),
    tc!(9, "000", "Lilac", 0xdd84cd),
    tc!(10, "502", "Mint Green", 0x6bd38a),
    tc!(11, "214", "Deep Gold", 0xe4a945),
    tc!(12, "208", "Orange", 0xffbd42),
    tc!(13, "000", "Yellow", 0xffe600),
    tc!(14, "513", "Lime Green", 0x6cd900),
    tc!(15, "328", "Brass", 0xc1a941),
    tc!(16, "005", "Silver", 0xb5ad97),
    tc!(17, "000", "Russet Brown", 0xba9c5f),
    tc!(18, "000", "Cream Brown", 0xfaf59e),
    tc!(19, "704", "Pewter", 0x808080),
    tc!(20, "900", "Black", 0x000000),
    tc!(21, "000", "Ultramarine", 0x001cdf),
    tc!(22, "000", "Royal Purple", 0xdf00b8),
    tc!(23, "707", "Dark Gray", 0x626262),
    tc!(24, "058", "Dark Brown", 0x69260d),
    tc!(25, "086", "Deep Rose", 0xff0060),
    tc!(26, "323", "Light Brown", 0xbf8200),
    tc!(27, "079", "Salmon Pink", 0xf39178),
    tc!(28, "000", "Vermilion", 0xff6805),
    tc!(29, "001", "White", 0xf0f0f0),
    tc!(30, "000", "Violet", 0xc832cd),
    tc!(31, "000", "Seacrest", 0xb0bf9b),
    tc!(32, "019", "Sky Blue", 0x65bfeb),
    tc!(33, "000", "Pumpkin", 0xffba04),
    tc!(34, "010", "Cream Yellow", 0xfff06c),
    tc!(35, "000", "Khaki", 0xfeca15),
    tc!(36, "000", "Clay Brown", 0xf38101),
    tc!(37, "000", "Leaf Green", 0x37a923),
    tc!(38, "405", "Peacock Blue", 0x23465f),
    tc!(39, "000", "Gray", 0xa6a695),
    tc!(40, "000", "Warm Gray", 0xcebfa6),
    tc!(41, "000", "Dark Olive", 0x96aa02),
    tc!(42, "307", "Linen", 0xffe3c6),
    tc!(43, "000", "Pink", 0xff99d7),
    tc!(44, "000", "Deep Green", 0x007004),
    tc!(45, "000", "Lavender", 0xedccfb),
    tc!(46, "000", "Wisteria Violet", 0xc089d8),
    tc!(47, "843", "Beige", 0xe7d9b4),
    tc!(48, "000", "Carmine", 0xe90e86),
    tc!(49, "000", "Amber Red", 0xcf6829),
    tc!(50, "000", "Olive Green", 0x408615),
    tc!(51, "107", "Dark Fuchsia", 0xdb1797),
    tc!(52, "209", "Tangerine", 0xffa704),
    tc!(53, "017", "Light Blue", 0xb9ffff),
    tc!(54, "507", "Emerald Green", 0x228927),
    tc!(55, "614", "Purple", 0xb612cd),
    tc!(56, "515", "Moss Green", 0x00aa00),
    tc!(57, "124", "Flesh Pink", 0xfea9dc),
    tc!(58, "000", "Harvest Gold", 0xfed510),
    tc!(59, "000", "Electric Blue", 0x0097df),
    tc!(60, "205", "Lemon Yellow", 0xffff84),
    tc!(61, "027", "Fresh Green", 0xcfe774),
    tc!(62, "000", "Applique Material", 0xffc864),
    tc!(63, "000", "Applique Position", 0xffc8c8),
    tc!(64, "000", "Applique", 0xffc8c8),
];

/// Looks up a Brother thread-table entry by its on-wire (1-based)
/// index.
pub fn brother_thread(index: u8) -> Option<&'static ThreadColor> {
    if (1..=64).contains(&index) {
        Some(&BROTHER_THREADS[index as usize - 1])
    } else {
        None
    }
}

/// A decoded PEC block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PecBlock {
    /// `LA:` label.
    pub label: String,
    /// Brother thread-table indices, one per colour block.
    pub palette: Vec<u8>,
    /// Design width from section 2 (extent span + 1).
    pub width: u16,
    /// Design height from section 2 (extent span + 1).
    pub height: u16,
    /// Raw 1-bit thumbnail images (228 bytes each).
    pub thumbnails: Vec<Vec<u8>>,
    /// The stitch design.
    pub design: Design,
    /// True when the stream opened with the ignorable `(0, 0)` pair.
    pub leading_origin_pair: bool,
}

/// Encoding options for a PEC block.
#[derive(Debug, Clone, Default)]
pub struct PecEncodeOptions {
    /// `LA:` label (up to 16 ASCII characters).
    pub label: String,
    /// Thread-table indices to store, one per colour block. When
    /// empty, indices are taken from `design.threads` palette
    /// indices, falling back to `1, 2, 3, …`.
    pub palette: Vec<u8>,
}

fn axis_short(b: u8) -> i32 {
    let v = (b & 0x7F) as i32;
    if v >= 0x40 {
        v - 0x80
    } else {
        v
    }
}

/// Per-axis decode: returns (delta, control-flag bits, bytes used).
fn decode_axis(data: &[u8], i: usize) -> Result<(i32, u8, usize)> {
    let b = *data.get(i).ok_or(Error::UnexpectedEof {
        context: "PEC stitch data",
    })?;
    if b & 0x80 != 0 {
        let b2 = *data.get(i + 1).ok_or(Error::UnexpectedEof {
            context: "PEC long-form delta",
        })?;
        let mut v = (((b & 0x0F) as i32) << 8) | b2 as i32;
        if v & 0x800 != 0 {
            v -= 0x1000;
        }
        Ok((v, b & 0x70, 2))
    } else {
        Ok((axis_short(b), 0, 1))
    }
}

/// Decodes a PEC stitch stream starting at `data[0]`. Returns the
/// commands, whether a leading `(0, 0)` pair was skipped, and the
/// total bytes consumed (including the terminating `0xFF`).
pub fn decode_stitches(data: &[u8]) -> Result<(Vec<Command>, bool, usize)> {
    let mut commands = Vec::new();
    let mut i = 0usize;
    let mut leading = false;
    // The documented ignorable opening (0, 0) pair.
    if data.len() >= 2 && data[0] == 0 && data[1] == 0 {
        leading = true;
        i = 2;
    }
    loop {
        let b = *data.get(i).ok_or(Error::UnexpectedEof {
            context: "PEC stitch data",
        })?;
        if b == 0xFF {
            i += 1;
            commands.push(Command::End);
            return Ok((commands, leading, i));
        }
        if b == 0xFE && data.get(i + 1) == Some(&0xB0) {
            let index = *data.get(i + 2).ok_or(Error::UnexpectedEof {
                context: "PEC colour-change index",
            })?;
            i += 3;
            commands.push(Command::ColorChange {
                dx: 0,
                dy: 0,
                index: Some(index),
            });
            continue;
        }
        let (dx, fx, nx) = decode_axis(data, i)?;
        i += nx;
        let (dy, fy, ny) = decode_axis(data, i)?;
        i += ny;
        let flags = fx | fy;
        if flags & 0x20 != 0 {
            commands.push(Command::Trim { dx, dy });
        } else if flags & 0x10 != 0 {
            commands.push(Command::Jump { dx, dy });
        } else {
            commands.push(Command::Stitch { dx, dy });
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum AxisKind {
    Plain,
    Jump,
    Trim,
}

fn push_axis(out: &mut Vec<u8>, v: i32, kind: AxisKind) {
    debug_assert!((MIN_DELTA..=MAX_DELTA).contains(&v));
    if kind == AxisKind::Plain && (-64..=63).contains(&v) {
        out.push((v & 0x7F) as u8);
    } else {
        let flag = match kind {
            AxisKind::Plain => 0x00,
            AxisKind::Jump => 0x10,
            AxisKind::Trim => 0x20,
        };
        let v12 = v & 0xFFF;
        out.push(0x80 | flag | ((v12 >> 8) as u8));
        out.push((v12 & 0xFF) as u8);
    }
}

fn push_pair(out: &mut Vec<u8>, dx: i32, dy: i32, kind: AxisKind) {
    // Jump/trim records carry the control flag on both axes, so both
    // axes use the long form.
    push_axis(out, dx, kind);
    push_axis(out, dy, kind);
}

fn push_move(out: &mut Vec<u8>, mut dx: i32, mut dy: i32, kind: AxisKind) {
    loop {
        let sx = dx.clamp(MIN_DELTA, MAX_DELTA);
        let sy = dy.clamp(MIN_DELTA, MAX_DELTA);
        push_pair(out, sx, sy, kind);
        dx -= sx;
        dy -= sy;
        if dx == 0 && dy == 0 {
            return;
        }
    }
}

/// Encodes a design's commands as a PEC stitch stream, including the
/// opening `(0, 0)` pair and the terminating `0xFF`.
pub fn encode_stitches(design: &Design) -> Result<Vec<u8>> {
    let mut out = vec![0x00, 0x00];
    let mut cc_index = 2u8;
    let mut ended = false;
    for c in &design.commands {
        if ended {
            break;
        }
        match *c {
            Command::Stitch { dx, dy } => push_move(&mut out, dx, dy, AxisKind::Plain),
            Command::Jump { dx, dy } => push_move(&mut out, dx, dy, AxisKind::Jump),
            Command::Trim { dx, dy } => push_move(&mut out, dx, dy, AxisKind::Trim),
            Command::ColorChange { dx, dy, index } => {
                out.push(0xFE);
                out.push(0xB0);
                out.push(index.unwrap_or(cc_index));
                cc_index = if cc_index == 2 { 1 } else { 2 };
                if dx != 0 || dy != 0 {
                    push_move(&mut out, dx, dy, AxisKind::Jump);
                }
            }
            Command::Stop => {
                return Err(Error::Unsupported {
                    what: "PEC has no stop record distinct from a colour change",
                });
            }
            Command::End => ended = true,
        }
    }
    out.push(0xFF);
    Ok(out)
}

fn render_thumbnail(design: &Design, color_block: Option<usize>, frame: bool) -> Vec<u8> {
    let mut img = vec![0u8; THUMB_LEN];
    let mut set = |x: i32, y: i32| {
        if (0..THUMB_WIDTH as i32).contains(&x) && (0..THUMB_HEIGHT as i32).contains(&y) {
            img[y as usize * THUMB_STRIDE + (x / 8) as usize] |= 0x80 >> (x % 8);
        }
    };
    if frame {
        for x in 0..THUMB_WIDTH as i32 {
            set(x, 0);
            set(x, THUMB_HEIGHT as i32 - 1);
        }
        for y in 0..THUMB_HEIGHT as i32 {
            set(0, y);
            set(THUMB_WIDTH as i32 - 1, y);
        }
    }
    let e = design.extents();
    let (w, h) = (e.width().max(1), e.height().max(1));
    let scale = |x: i32, y: i32| {
        let px = (x - e.min_x) as i64 * (THUMB_WIDTH as i64 - 5) / w as i64 + 2;
        let py = (y - e.min_y) as i64 * (THUMB_HEIGHT as i64 - 5) / h as i64 + 2;
        (px as i32, py as i32)
    };
    let (mut x, mut y) = (0i32, 0i32);
    let mut block = 0usize;
    for c in &design.commands {
        let (dx, dy) = c.delta();
        x += dx;
        y += dy;
        if let Command::ColorChange { .. } = c {
            block += 1;
            continue;
        }
        if let Command::Stitch { .. } = c {
            if color_block.map_or(true, |b| b == block) {
                let (px, py) = scale(x, y);
                set(px, py);
            }
        }
    }
    img
}

/// Encodes a design as a PEC block (the bytes a `.pes`/`.phc`
/// container embeds; prepend [`MAGIC`] for a standalone `.pec`).
pub fn encode_block(design: &Design, options: &PecEncodeOptions) -> Result<Vec<u8>> {
    if options.label.len() > 16 || !options.label.is_ascii() {
        return Err(Error::OutOfRange { field: "PEC label" });
    }
    let blocks = design.color_block_count().max(1);
    let palette: Vec<u8> = if !options.palette.is_empty() {
        options.palette.clone()
    } else if design.threads.len() >= blocks {
        design
            .threads
            .iter()
            .take(blocks)
            .enumerate()
            .map(|(i, t)| t.palette_index.map(|v| v as u8).unwrap_or(i as u8 + 1))
            .collect()
    } else {
        (1..=blocks as u8).collect()
    };
    if palette.len() != blocks {
        return Err(Error::invalid(format!(
            "palette has {} entries for {} colour blocks",
            palette.len(),
            blocks
        )));
    }
    if blocks > 0x100 {
        return Err(Error::OutOfRange {
            field: "PEC colour count",
        });
    }

    let pre = design.extents();
    if pre.width() >= 0xFFFF || pre.height() >= 0xFFFF {
        return Err(Error::OutOfRange {
            field: "PEC design size (section-2 width/height are u16)",
        });
    }

    let stitches = encode_stitches(design)?;
    if SECTION2_LEN + stitches.len() > 0xFF_FFFF {
        return Err(Error::OutOfRange {
            field: "PEC stitch data (thumbnail pointer is 3 bytes)",
        });
    }

    let mut sec1 = Vec::with_capacity(SECTION1_LEN);
    sec1.extend_from_slice(format!("LA:{:<16}\r", options.label).as_bytes());
    sec1.resize(48, 0x20);
    sec1.push((blocks - 1) as u8);
    sec1.extend_from_slice(&palette);
    sec1.resize(SECTION1_LEN, 0x20);

    let thumb_offset = (SECTION2_LEN + stitches.len()) as u32;
    let e = design.extents();
    let mut sec2 = Vec::with_capacity(SECTION2_LEN);
    sec2.extend_from_slice(&[0x00, 0x00]);
    sec2.extend_from_slice(&thumb_offset.to_le_bytes()[..3]);
    sec2.extend_from_slice(&[0x00; 3]);
    sec2.extend_from_slice(&(e.width() as u16 + 1).to_le_bytes());
    sec2.extend_from_slice(&(e.height() as u16 + 1).to_le_bytes());
    // Observed-constant pair (480, 432) from the validation corpus.
    sec2.extend_from_slice(&480u16.to_le_bytes());
    sec2.extend_from_slice(&432u16.to_le_bytes());
    sec2.extend_from_slice(&[0x00; 4]);
    debug_assert_eq!(sec2.len(), SECTION2_LEN);

    let mut out = sec1;
    out.extend_from_slice(&sec2);
    out.extend_from_slice(&stitches);
    out.extend_from_slice(&render_thumbnail(design, None, true));
    for b in 0..blocks {
        out.extend_from_slice(&render_thumbnail(design, Some(b), false));
    }
    Ok(out)
}

/// Decodes a PEC block (starting at section 1).
pub fn decode_block(block: &[u8]) -> Result<PecBlock> {
    if block.len() < STITCH_OFFSET + 1 {
        return Err(Error::UnexpectedEof {
            context: "PEC block header",
        });
    }
    if &block[..3] != b"LA:" {
        return Err(Error::BadMagic { expected: "PEC" });
    }
    let label = String::from_utf8_lossy(&block[3..19])
        .trim_end_matches([' ', '\0'])
        .to_string();
    let colors = block[48] as usize + 1;
    if 49 + colors > SECTION1_LEN {
        return Err(Error::invalid("PEC colour count overruns section 1"));
    }
    let palette = block[49..49 + colors].to_vec();

    let sec2 = &block[SECTION1_LEN..STITCH_OFFSET];
    let thumb_offset = u32::from_le_bytes([sec2[2], sec2[3], sec2[4], 0]) as usize + SECTION1_LEN;
    let width = u16::from_le_bytes([sec2[8], sec2[9]]);
    let height = u16::from_le_bytes([sec2[10], sec2[11]]);

    let (commands, leading, _used) = decode_stitches(&block[STITCH_OFFSET..])?;

    let mut thumbnails = Vec::new();
    if thumb_offset >= STITCH_OFFSET {
        let mut t = thumb_offset;
        for _ in 0..=colors {
            if t + THUMB_LEN > block.len() {
                break;
            }
            thumbnails.push(block[t..t + THUMB_LEN].to_vec());
            t += THUMB_LEN;
        }
    }

    let threads = palette
        .iter()
        .map(|&idx| {
            let known = brother_thread(idx);
            Thread {
                palette_index: Some(idx as u16),
                rgb: known.map(|k| k.rgb),
                catalog: known.map(|k| k.code.to_string()),
                name: known.map(|k| k.name.to_string()),
            }
        })
        .collect();

    Ok(PecBlock {
        label: label.clone(),
        palette,
        width,
        height,
        thumbnails,
        design: Design {
            commands,
            threads,
            label: if label.is_empty() { None } else { Some(label) },
        },
        leading_origin_pair: leading,
    })
}

/// Returns true when `data` starts with the standalone `.pec`
/// signature.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 8 && &data[..4] == b"#PEC"
}

/// Decodes a standalone `.pec` file (`#PEC0001` + block).
pub fn decode(data: &[u8]) -> Result<PecBlock> {
    if data.len() < 8 || &data[..4] != b"#PEC" {
        return Err(Error::BadMagic { expected: "PEC" });
    }
    decode_block(&data[8..])
}

/// Encodes a standalone `.pec` file.
pub fn encode(design: &Design, options: &PecEncodeOptions) -> Result<Vec<u8>> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&encode_block(design, options)?);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Design};

    #[test]
    fn short_form_axis() {
        // 0–63 positive, 64–127 negative (7-bit two's complement).
        assert_eq!(axis_short(0), 0);
        assert_eq!(axis_short(63), 63);
        assert_eq!(axis_short(64), -64);
        assert_eq!(axis_short(127), -1);
    }

    #[test]
    fn long_form_negative_matches_staged_formula() {
        // Staged doc: negative jump length = (ky−256) + ((kx&15)−15)×256.
        // kx = 0x9F, ky = 0xFF → −1 with the jump flag.
        let (v, flags, n) = decode_axis(&[0x9F, 0xFF], 0).unwrap();
        assert_eq!((v, flags, n), (-1, 0x10, 2));
        // kx = 0x98, ky = 0x00 → (0−256) + (8−15)×256 = −2048.
        let (v, _, _) = decode_axis(&[0x98, 0x00], 0).unwrap();
        assert_eq!(v, -2048);
        // Positive: length = ky + (kx&15)×256; kx = 0x93, ky = 0x21.
        let (v, _, _) = decode_axis(&[0x93, 0x21], 0).unwrap();
        assert_eq!(v, 0x321);
    }

    #[test]
    fn axis_roundtrip_exhaustive() {
        for v in MIN_DELTA..=MAX_DELTA {
            for kind in [AxisKind::Plain, AxisKind::Jump, AxisKind::Trim] {
                let mut out = Vec::new();
                push_axis(&mut out, v, kind);
                let (got, flags, used) = decode_axis(&out, 0).unwrap();
                assert_eq!(got, v);
                assert_eq!(used, out.len());
                match kind {
                    AxisKind::Plain => assert_eq!(flags, 0),
                    AxisKind::Jump => {
                        if used == 2 {
                            assert_eq!(flags, 0x10)
                        }
                    }
                    AxisKind::Trim => {
                        if used == 2 {
                            assert_eq!(flags, 0x20)
                        }
                    }
                }
            }
        }
    }

    fn sample_design() -> Design {
        Design {
            commands: vec![
                Command::Jump { dx: 200, dy: -100 },
                Command::Stitch { dx: 30, dy: 30 },
                Command::Stitch { dx: -63, dy: 63 },
                Command::Trim { dx: 0, dy: 0 },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::Stitch { dx: 100, dy: -2000 },
                Command::Stitch { dx: 1, dy: 1 },
                Command::End,
            ],
            threads: Vec::new(),
            label: None,
        }
    }

    #[test]
    fn stitch_stream_roundtrip() {
        let d = sample_design();
        let bytes = encode_stitches(&d).unwrap();
        assert_eq!(bytes[..2], [0x00, 0x00]);
        assert_eq!(*bytes.last().unwrap(), 0xFF);
        let (commands, leading, used) = decode_stitches(&bytes).unwrap();
        assert!(leading);
        assert_eq!(used, bytes.len());
        // Colour-change index bytes are materialised on decode.
        let expected: Vec<Command> = d
            .commands
            .iter()
            .map(|c| match *c {
                Command::ColorChange { dx, dy, index } => Command::ColorChange {
                    dx,
                    dy,
                    index: Some(index.unwrap_or(2)),
                },
                other => other,
            })
            .collect();
        assert_eq!(commands, expected);
    }

    #[test]
    fn color_change_index_alternates() {
        let d = Design {
            commands: vec![
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::End,
            ],
            ..Default::default()
        };
        let bytes = encode_stitches(&d).unwrap();
        // 00 00, FE B0 02, FE B0 01, FE B0 02, FF
        assert_eq!(
            bytes,
            vec![0, 0, 0xFE, 0xB0, 2, 0xFE, 0xB0, 1, 0xFE, 0xB0, 2, 0xFF]
        );
    }

    #[test]
    fn block_roundtrip() {
        let d = sample_design();
        let opts = PecEncodeOptions {
            label: "BLOCK".into(),
            palette: vec![20, 5],
        };
        let block = encode_block(&d, &opts).unwrap();
        let p = decode_block(&block).unwrap();
        assert_eq!(p.label, "BLOCK");
        assert_eq!(p.palette, vec![20, 5]);
        // Black + Red from the thread table.
        assert_eq!(p.design.threads[0].name.as_deref(), Some("Black"));
        assert_eq!(p.design.threads[0].rgb, Some([0, 0, 0]));
        assert_eq!(p.design.threads[1].name.as_deref(), Some("Red"));
        let e = d.extents();
        assert_eq!(p.width as i32, e.width() + 1);
        assert_eq!(p.height as i32, e.height() + 1);
        // Master + one per colour block.
        assert_eq!(p.thumbnails.len(), 3);
        assert!(p.thumbnails.iter().all(|t| t.len() == THUMB_LEN));
        assert_eq!(p.design.counts().stitches, d.counts().stitches);
        assert_eq!(p.design.counts().trims, 1);
        assert_eq!(p.design.counts().color_changes, 1);
    }

    #[test]
    fn standalone_roundtrip_and_documented_stitch_offset() {
        let d = sample_design();
        let bytes = encode(&d, &PecEncodeOptions::default()).unwrap();
        assert!(probe(&bytes));
        assert_eq!(&bytes[..8], MAGIC);
        // Corpus-validated: standalone stitch list starts at byte
        // 8 + 512 + 20 = 540, opening with the (0,0) pair.
        assert_eq!(bytes[540], 0x00);
        assert_eq!(bytes[541], 0x00);
        let p = decode(&bytes).unwrap();
        assert!(p.leading_origin_pair);
        assert_eq!(p.design.counts(), d.counts());
    }

    #[test]
    fn thread_table_bounds() {
        assert_eq!(brother_thread(0), None);
        assert_eq!(brother_thread(65), None);
        assert_eq!(brother_thread(1).unwrap().name, "Prussian Blue");
        assert_eq!(brother_thread(64).unwrap().name, "Applique");
        for (i, t) in BROTHER_THREADS.iter().enumerate() {
            assert_eq!(t.index as usize, i + 1);
        }
    }

    #[test]
    fn truncated_stream_errors() {
        assert!(matches!(
            decode_stitches(&[0x00, 0x00, 0x12]),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}
