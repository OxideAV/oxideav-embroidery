//! Tajima DST — decode and encode.
//!
//! Layout per the workspace's staged documentation
//! (`docs/embroidery/dst/`): a 512-byte ASCII header (label, counts,
//! extents, vestigial tape-splicing fields, closed by `0x1A` at
//! offset 124), then 3-byte stitch records whose x/y deltas are
//! ternary-coded bit weights (±1, ±3, ±9, ±27, ±81 per axis, so
//! ±121 per record), and a `00 00 F3` end record.
//!
//! Header numeric-field conventions, corpus-pinned (8 real files, no
//! variance): values are **space-padded and right-aligned, never
//! zero-padded**; the `0x1A` terminator sits at offset 124 and the
//! tail is `0x20`-filled to 512. The signed `AX/AY/MX/MY` fields
//! carry the sign character first (`+` on zero) followed by the
//! space-padded magnitude, and an unused `PD:` is `******`. The one
//! deviating writer in the corpus is a documented variant (`PD:`
//! dashes, zero-padded `MX`, negative-signed zero `AX`) that the
//! decoder must and does accept; a reader must not assume `+` on
//! zero.

use crate::model::{Command, Design, Diagnostics, WarningAction};
use crate::{Error, Result};

/// Byte length of the DST header.
pub const HEADER_LEN: usize = 512;
/// Maximum per-axis delta a single 3-byte record can carry.
pub const MAX_DELTA: i32 = 121;

/// Parsed DST header fields. Numeric fields are `None` when the
/// writer left them unpopulated or unparsable — everything except
/// the label is redundant with the stitch data and frequently wrong
/// in third-party files, per the staged documentation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DstHeader {
    /// `LA:` design label (up to 16 characters).
    pub label: String,
    /// `ST:` total record count (stitches + jumps + colour changes +
    /// the end record).
    pub stitch_records: Option<u32>,
    /// `CO:` colour-change count.
    pub color_changes: Option<u32>,
    /// `+X:` extent magnitude, 0.1 mm.
    pub pos_x: Option<i32>,
    /// `-X:` extent magnitude, 0.1 mm.
    pub neg_x: Option<i32>,
    /// `+Y:` extent magnitude, 0.1 mm.
    pub pos_y: Option<i32>,
    /// `-Y:` extent magnitude, 0.1 mm.
    pub neg_y: Option<i32>,
    /// `AX:` needle end point, signed.
    pub ax: Option<i32>,
    /// `AY:` needle end point, signed.
    pub ay: Option<i32>,
    /// `MX:` previous-file needle end point (vestigial).
    pub mx: Option<i32>,
    /// `MY:` previous-file needle end point (vestigial).
    pub my: Option<i32>,
    /// `PD:` previous-file link (vestigial), raw text.
    pub pd: Option<String>,
}

/// A decoded DST file: the parsed header plus the stitch design.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DstFile {
    pub header: DstHeader,
    pub design: Design,
    /// True when the `00 00 F3` end record was present.
    pub terminated: bool,
    /// Number of trailing bytes after the end record that do not form
    /// a whole record (`1` for files written by the digitiser variant
    /// with the documented `(size − 512) % 3 == 1` quirk).
    pub trailing_bytes: usize,
}

/// Encoding options.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DstEncodeOptions {
    /// Design label for the `LA:` field (up to 16 ASCII characters).
    pub label: String,
    /// Express [`Command::Trim`] as the documented
    /// trim-by-consecutive-jumps convention: a deliberate no-op jump
    /// run (+2+2, −4−4, +2+2) that machines recognise as a trim
    /// request. When `false`, a trim is encoded as a single jump.
    pub trim_as_jump_run: bool,
}

impl DstEncodeOptions {
    /// Set `label`.
    pub fn with_label(mut self, label: String) -> Self {
        self.label = label;
        self
    }

    /// Set `trim_as_jump_run`.
    pub fn with_trim_as_jump_run(mut self, trim_as_jump_run: bool) -> Self {
        self.trim_as_jump_run = trim_as_jump_run;
        self
    }
}

impl Default for DstEncodeOptions {
    fn default() -> Self {
        DstEncodeOptions {
            label: String::new(),
            trim_as_jump_run: true,
        }
    }
}

/// Returns true when `data` plausibly starts a DST file: a `LA:`
/// header lead-in and room for the 512-byte header.
pub fn probe(data: &[u8]) -> bool {
    data.len() > HEADER_LEN && data.starts_with(b"LA:")
}

// Field layout: (prefix, value width). Fields are back-to-back from
// offset 0 and total exactly 124 bytes including each CR terminator.
const FIELDS: [(&str, usize); 12] = [
    ("LA:", 16),
    ("ST:", 7),
    ("CO:", 3),
    ("+X:", 5),
    ("-X:", 5),
    ("+Y:", 5),
    ("-Y:", 5),
    ("AX:", 6),
    ("AY:", 6),
    ("MX:", 6),
    ("MY:", 6),
    ("PD:", 6),
];

fn parse_num(raw: &str) -> Option<i32> {
    let t = raw.trim_matches(|c: char| c == ' ' || c == '\0');
    if t.is_empty() {
        return None;
    }
    // The signed fields put the sign character first and space-pad
    // between it and the digits (`+    0`); one corpus writer also
    // zero-pads (`+   00`) and writes negative-signed zeros
    // (`-    0`). Accept all of these.
    let (sign, digits) = match t.as_bytes()[0] {
        b'+' => (1, t[1..].trim_start()),
        b'-' => (-1, t[1..].trim_start()),
        _ => (1, t),
    };
    digits.parse::<i32>().ok().map(|v| sign * v)
}

/// Parses the 512-byte ASCII header alone.
///
/// Exposed separately because, per the staged corpus mapping
/// (`docs/embroidery/corpus-map.md`), the Barudan `.dsb` and ZSK
/// `.dsz` tape formats carry the **identical** `LA:`-prefixed
/// 512-byte header while their 3-byte stitch records use different,
/// undocumented bit assignments (the ternary table decodes them to
/// nonsense geometry — a validated negative). For those files this
/// header parse is the full documented depth; do not run
/// [`decode`]'s record loop on them.
pub fn decode_header(data: &[u8]) -> Result<DstHeader> {
    if data.len() < HEADER_LEN {
        return Err(Error::UnexpectedEof {
            context: "DST header",
        });
    }
    let mut header = DstHeader::default();
    let mut off = 0usize;
    let mut values: Vec<Option<String>> = Vec::with_capacity(FIELDS.len());
    for (prefix, width) in FIELDS {
        let end = off + prefix.len() + width + 1;
        let field = &data[off..end];
        if &field[..prefix.len()] == prefix.as_bytes() {
            let raw = &field[prefix.len()..prefix.len() + width];
            values.push(Some(String::from_utf8_lossy(raw).into_owned()));
        } else {
            values.push(None);
        }
        off = end;
    }
    debug_assert_eq!(off, 124);
    if values[0].is_none() {
        return Err(Error::BadMagic { expected: "DST" });
    }
    header.label = values[0]
        .as_deref()
        .unwrap_or("")
        .trim_end_matches([' ', '\0'])
        .to_string();
    header.stitch_records = values[1].as_deref().and_then(parse_num).map(|v| v as u32);
    header.color_changes = values[2].as_deref().and_then(parse_num).map(|v| v as u32);
    header.pos_x = values[3].as_deref().and_then(parse_num);
    header.neg_x = values[4].as_deref().and_then(parse_num);
    header.pos_y = values[5].as_deref().and_then(parse_num);
    header.neg_y = values[6].as_deref().and_then(parse_num);
    header.ax = values[7].as_deref().and_then(parse_num);
    header.ay = values[8].as_deref().and_then(parse_num);
    header.mx = values[9].as_deref().and_then(parse_num);
    header.my = values[10].as_deref().and_then(parse_num);
    header.pd = values[11].as_ref().map(|s| s.trim().to_string());
    Ok(header)
}

/// Decodes a DST file.
pub fn decode(data: &[u8]) -> Result<DstFile> {
    let header = decode_header(data)?;

    let body = &data[HEADER_LEN..];
    let mut commands = Vec::with_capacity(body.len() / 3 + 1);
    let mut diagnostics = Diagnostics::default();
    let mut terminated = false;
    let mut trailing = 0usize;
    let mut i = 0usize;
    while i + 3 <= body.len() {
        let (b1, b2, b3) = (body[i], body[i + 1], body[i + 2]);
        i += 3;
        if b3 == 0xF3 {
            terminated = true;
            commands.push(Command::End);
            trailing = body.len() - i;
            break;
        }
        let (dx, dy) = decode_delta(b1, b2, b3);
        // Outside the documented vocabulary: bit 6 without bit 7 (a
        // colour change sets both; the lone bit has no file-backed
        // meaning) and a record without the two always-set low
        // bits. The ternary table still defines the movement, so the
        // record keeps its historical reading and is reported.
        if b3 & 0xC0 == 0x40 || b3 & 0x03 != 0x03 {
            diagnostics.push(
                "DST",
                HEADER_LEN + i - 3,
                b3,
                [b1, b2],
                WarningAction::DecodedByRule,
            );
        }
        if b3 & 0xC0 == 0xC0 {
            commands.push(Command::ColorChange {
                dx,
                dy,
                index: None,
            });
        } else if b3 & 0x80 != 0 {
            commands.push(Command::Jump { dx, dy });
        } else {
            commands.push(Command::Stitch { dx, dy });
        }
    }
    if !terminated {
        trailing = body.len() - i;
    }
    let label = if header.label.is_empty() {
        None
    } else {
        Some(header.label.clone())
    };
    Ok(DstFile {
        header,
        design: Design {
            commands,
            threads: Vec::new(),
            label,
            diagnostics,
        },
        terminated,
        trailing_bytes: trailing,
    })
}

/// The ternary bit table: `(byte, bit mask, x weight, y weight)` for
/// each of the twenty movement bits of a record.
const MOVE_BITS: [(usize, u8, i8, i8); 20] = [
    // byte 1: y±1, y±9, x∓9, x∓1
    (0, 0x80, 0, 1),
    (0, 0x40, 0, -1),
    (0, 0x20, 0, 9),
    (0, 0x10, 0, -9),
    (0, 0x08, -9, 0),
    (0, 0x04, 9, 0),
    (0, 0x02, -1, 0),
    (0, 0x01, 1, 0),
    // byte 2: the same pattern ×3
    (1, 0x80, 0, 3),
    (1, 0x40, 0, -3),
    (1, 0x20, 0, 27),
    (1, 0x10, 0, -27),
    (1, 0x08, -27, 0),
    (1, 0x04, 27, 0),
    (1, 0x02, -3, 0),
    (1, 0x01, 3, 0),
    // byte 3: y±81 (bits 5/4), x∓81 (bits 3/2)
    (2, 0x20, 0, 81),
    (2, 0x10, 0, -81),
    (2, 0x08, -81, 0),
    (2, 0x04, 81, 0),
];

/// Per record byte position and byte value, the `(x, y)` movement
/// that byte contributes — [`MOVE_BITS`] summed over the set bits.
const BYTE_MOVES: [[(i8, i8); 256]; 3] = {
    let mut table = [[(0i8, 0i8); 256]; 3];
    let mut value = 0usize;
    while value < 256 {
        let mut k = 0usize;
        while k < MOVE_BITS.len() {
            let (byte, mask, wx, wy) = MOVE_BITS[k];
            if value as u8 & mask != 0 {
                table[byte][value].0 += wx;
                table[byte][value].1 += wy;
            }
            k += 1;
        }
        value += 1;
    }
    table
};

/// Decodes the x/y delta of one 3-byte record per the ternary bit
/// table.
fn decode_delta(b1: u8, b2: u8, b3: u8) -> (i32, i32) {
    let (x1, y1) = BYTE_MOVES[0][b1 as usize];
    let (x2, y2) = BYTE_MOVES[1][b2 as usize];
    let (x3, y3) = BYTE_MOVES[2][b3 as usize];
    (
        x1 as i32 + x2 as i32 + x3 as i32,
        y1 as i32 + y2 as i32 + y3 as i32,
    )
}

/// Balanced-ternary digits (−1/0/+1) for weights 1, 3, 9, 27, 81.
/// Unique for every value in [−121, 121].
const fn trits(mut v: i32) -> [i8; 5] {
    let mut t = [0i8; 5];
    let mut k = 0usize;
    while k < 5 {
        let r = v.rem_euclid(3);
        if r == 2 {
            t[k] = -1;
            v = (v + 1) / 3;
        } else {
            t[k] = r as i8;
            v = (v - r) / 3;
        }
        k += 1;
    }
    t
}

/// The three record bytes carrying a single-axis delta: the bit of
/// [`MOVE_BITS`] with weight `digit × 3^k` set for every non-zero
/// balanced-ternary digit of the delta.
const fn axis_bytes(delta: i32, x_axis: bool) -> [u8; 3] {
    let digits = trits(delta);
    let mut bytes = [0u8; 3];
    let mut weight = 1i8;
    let mut k = 0usize;
    while k < 5 {
        if digits[k] != 0 {
            let want = digits[k] * weight;
            let mut m = 0usize;
            while m < MOVE_BITS.len() {
                let (byte, mask, wx, wy) = MOVE_BITS[m];
                if (x_axis && wx == want) || (!x_axis && wy == want) {
                    bytes[byte] |= mask;
                }
                m += 1;
            }
        }
        if k < 4 {
            weight *= 3;
        }
        k += 1;
    }
    bytes
}

/// Record bytes per axis delta, indexed by `delta + MAX_DELTA`.
const AXIS_TABLE_LEN: usize = 2 * MAX_DELTA as usize + 1;
const fn axis_table(x_axis: bool) -> [[u8; 3]; AXIS_TABLE_LEN] {
    let mut table = [[0u8; 3]; AXIS_TABLE_LEN];
    let mut i = 0usize;
    while i < AXIS_TABLE_LEN {
        table[i] = axis_bytes(i as i32 - MAX_DELTA, x_axis);
        i += 1;
    }
    table
}
const X_BYTES: [[u8; 3]; AXIS_TABLE_LEN] = axis_table(true);
const Y_BYTES: [[u8; 3]; AXIS_TABLE_LEN] = axis_table(false);

/// Encodes one record's delta into the three record bytes (control
/// bits not included).
fn encode_delta(dx: i32, dy: i32) -> (u8, u8, u8) {
    debug_assert!((-MAX_DELTA..=MAX_DELTA).contains(&dx));
    debug_assert!((-MAX_DELTA..=MAX_DELTA).contains(&dy));
    let x = X_BYTES[(dx + MAX_DELTA) as usize];
    let y = Y_BYTES[(dy + MAX_DELTA) as usize];
    (x[0] | y[0], x[1] | y[1], x[2] | y[2])
}

#[derive(Clone, Copy, PartialEq)]
enum RecKind {
    Stitch,
    Jump,
    ColorChange,
}

fn push_record(out: &mut Vec<u8>, dx: i32, dy: i32, kind: RecKind) {
    let (b1, b2, mut b3) = encode_delta(dx, dy);
    b3 |= 0x03;
    match kind {
        RecKind::Stitch => {}
        RecKind::Jump => b3 |= 0x80,
        RecKind::ColorChange => b3 |= 0xC0,
    }
    out.extend_from_slice(&[b1, b2, b3]);
}

/// Splits a move into records of at most ±121 per axis, all of the
/// same kind.
fn push_move(out: &mut Vec<u8>, mut dx: i32, mut dy: i32, kind: RecKind) -> usize {
    let mut n = 0usize;
    loop {
        let sx = dx.clamp(-MAX_DELTA, MAX_DELTA);
        let sy = dy.clamp(-MAX_DELTA, MAX_DELTA);
        push_record(out, sx, sy, kind);
        n += 1;
        dx -= sx;
        dy -= sy;
        if dx == 0 && dy == 0 {
            return n;
        }
    }
}

/// Encodes a design as a DST file.
///
/// Moves larger than ±121 units per axis are split into several
/// records of the same kind. [`Command::Trim`] is expressed per
/// `options.trim_as_jump_run`; [`Command::Stop`] has no DST encoding
/// and returns [`Error::Unsupported`].
pub fn encode(design: &Design, options: &DstEncodeOptions) -> Result<Vec<u8>> {
    if options.label.len() > 16 || !options.label.is_ascii() {
        return Err(Error::OutOfRange { field: "DST label" });
    }
    // The header's extent fields are 5 digits and AX/AY are a sign
    // plus 5 digits, so every needle position must stay within
    // ±99999 units — this also bounds the record-splitting loops.
    let pre = design.extents();
    if pre.min_x < -99_999 || pre.max_x > 99_999 || pre.min_y < -99_999 || pre.max_y > 99_999 {
        return Err(Error::OutOfRange {
            field: "DST extents (header fields are 5 digits)",
        });
    }
    // One allocation for header + body; a record per command is the
    // common case (long moves and trim runs grow it).
    let mut body = Vec::with_capacity(HEADER_LEN + 3 * design.commands.len() + 3);
    body.resize(HEADER_LEN, b' ');
    let mut records = 0usize;
    let mut color_changes = 0usize;
    let mut ended = false;
    for c in &design.commands {
        if ended {
            break;
        }
        match *c {
            Command::Stitch { dx, dy } => records += push_move(&mut body, dx, dy, RecKind::Stitch),
            Command::Jump { dx, dy } => records += push_move(&mut body, dx, dy, RecKind::Jump),
            Command::Trim { dx, dy } => {
                if options.trim_as_jump_run {
                    // Documented deliberate no-op jump run that
                    // machines recognise as a trim request.
                    records += push_move(&mut body, 2, 2, RecKind::Jump);
                    records += push_move(&mut body, -4, -4, RecKind::Jump);
                    records += push_move(&mut body, 2, 2, RecKind::Jump);
                }
                if dx != 0 || dy != 0 || !options.trim_as_jump_run {
                    records += push_move(&mut body, dx, dy, RecKind::Jump);
                }
            }
            Command::ColorChange { dx, dy, .. } => {
                records += push_move(&mut body, dx, dy, RecKind::ColorChange);
                color_changes += 1;
            }
            Command::Stop => {
                return Err(Error::Unsupported {
                    what: "DST has no stop record distinct from a colour change",
                });
            }
            Command::End => ended = true,
        }
    }
    body.extend_from_slice(&[0x00, 0x00, 0xF3]);
    records += 1;

    // The pre-check's extents are the design's; the needle end
    // point is the sum of all deltas (in range per that check).
    let e = pre;
    let end = design.commands.iter().fold((0i32, 0i32), |(x, y), c| {
        let (dx, dy) = c.delta();
        (x + dx, y + dy)
    });
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(format!("LA:{:<16}\r", options.label).as_bytes());
    header.extend_from_slice(format!("ST:{records:7}\r").as_bytes());
    header.extend_from_slice(format!("CO:{color_changes:3}\r").as_bytes());
    header.extend_from_slice(format!("+X:{:5}\r", e.max_x.max(0)).as_bytes());
    header.extend_from_slice(format!("-X:{:5}\r", (-e.min_x).max(0)).as_bytes());
    header.extend_from_slice(format!("+Y:{:5}\r", e.max_y.max(0)).as_bytes());
    header.extend_from_slice(format!("-Y:{:5}\r", (-e.min_y).max(0)).as_bytes());
    // Signed fields: sign character first (`+` on zero), then the
    // space-padded magnitude — the corpus-pinned standard form.
    let signed = |v: i32| format!("{}{:5}", if v < 0 { '-' } else { '+' }, v.abs());
    header.extend_from_slice(format!("AX:{}\r", signed(end.0)).as_bytes());
    header.extend_from_slice(format!("AY:{}\r", signed(end.1)).as_bytes());
    header.extend_from_slice(format!("MX:{}\r", signed(0)).as_bytes());
    header.extend_from_slice(format!("MY:{}\r", signed(0)).as_bytes());
    header.extend_from_slice(b"PD:******\r");
    debug_assert_eq!(header.len(), 124);
    header.push(0x1A);
    body[..header.len()].copy_from_slice(&header);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Design, Thread};

    // Worked examples from the staged Grade-A bit table
    // (docs/embroidery/dst/achatina-dst-design-format.html).
    #[test]
    fn worked_example_stitch() {
        assert_eq!(decode_delta(0xA8, 0x12, 0x23), (-12, 64));
    }

    #[test]
    fn worked_example_jump_flag() {
        // A8 12 A3 — same delta, jump bit set.
        assert_eq!(0xA3 & 0xC0, 0x80);
        assert_eq!(decode_delta(0xA8, 0x12, 0xA3), (-12, 64));
    }

    #[test]
    fn undocumented_control_bits_are_reported_not_silent() {
        let d = sample_design();
        let mut bytes = encode(&d, &DstEncodeOptions::default()).unwrap();
        let clean = decode(&bytes).unwrap();
        assert!(clean.design.diagnostics.is_empty());
        // Bit 6 without bit 7 on the first record, and a second
        // record without the always-set low bits.
        let (b1, b2) = (bytes[HEADER_LEN], bytes[HEADER_LEN + 1]);
        bytes[HEADER_LEN + 2] = (bytes[HEADER_LEN + 2] & 0x3F) | 0x40;
        bytes[HEADER_LEN + 5] &= 0xFC;
        let got = decode(&bytes).unwrap();
        let w = got.design.diagnostics.warnings();
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].format, w[0].offset), ("DST", HEADER_LEN));
        assert_eq!(w[0].code & 0xC0, 0x40);
        assert_eq!(w[0].payload, [b1, b2]);
        assert_eq!(w[0].action, WarningAction::DecodedByRule);
        assert_eq!(w[1].offset, HEADER_LEN + 3);
        // The ternary table still defines the movement.
        assert_eq!(
            got.design.positions().last(),
            clean.design.positions().last()
        );
        assert!(matches!(
            crate::DecodeOptions::strict().check(&got.design),
            Err(Error::UnknownControl { format: "DST", .. })
        ));
    }

    #[test]
    fn worked_example_color_change() {
        // 00 00 C3 — colour change.
        assert_eq!(0xC3 & 0xC0, 0xC0);
        assert_eq!(decode_delta(0x00, 0x00, 0xC3), (0, 0));
    }

    #[test]
    fn delta_roundtrip_exhaustive() {
        for dx in -121..=121 {
            for dy in [-121, -80, -13, -1, 0, 1, 40, 121] {
                let (b1, b2, b3) = encode_delta(dx, dy);
                assert_eq!(decode_delta(b1, b2, b3 | 0x03), (dx, dy), "({dx},{dy})");
            }
        }
    }

    #[test]
    fn encoded_stitch_matches_worked_example() {
        let mut out = Vec::new();
        push_record(&mut out, -12, 64, RecKind::Stitch);
        assert_eq!(out, vec![0xA8, 0x12, 0x23]);
    }

    fn sample_design() -> Design {
        Design {
            commands: vec![
                Command::Jump { dx: 100, dy: -50 },
                Command::Stitch { dx: 5, dy: 5 },
                Command::Stitch { dx: -5, dy: 5 },
                Command::Stitch { dx: 121, dy: -121 },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::Stitch { dx: 30, dy: 40 },
                Command::End,
            ],
            threads: vec![Thread::default(), Thread::default()],
            label: None,
            ..Default::default()
        }
    }

    #[test]
    fn roundtrip() {
        let d = sample_design();
        let bytes = encode(
            &d,
            &DstEncodeOptions {
                label: "RT".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(probe(&bytes));
        assert_eq!(bytes[124], 0x1A);
        assert_eq!((bytes.len() - HEADER_LEN) % 3, 0);
        let f = decode(&bytes).unwrap();
        assert!(f.terminated);
        assert_eq!(f.trailing_bytes, 0);
        assert_eq!(f.header.label, "RT");
        assert_eq!(f.header.color_changes, Some(1));
        // 6 body records + end record.
        assert_eq!(f.header.stitch_records, Some(7));
        assert_eq!(f.design.commands, d.commands);
        let e = d.extents();
        assert_eq!(f.header.pos_x, Some(e.max_x.max(0)));
        assert_eq!(f.header.neg_x, Some((-e.min_x).max(0)));
        assert_eq!(f.header.pos_y, Some(e.max_y.max(0)));
        assert_eq!(f.header.neg_y, Some((-e.min_y).max(0)));
    }

    #[test]
    fn header_numeric_fields_use_pinned_padding() {
        // Corpus-pinned forms: space-padded right-aligned counts and
        // extents, sign-first signed fields with `+` on zero, and an
        // unused `PD:` written as asterisks.
        let d = Design {
            commands: vec![
                Command::Stitch { dx: 30, dy: 40 },
                Command::Stitch { dx: -30, dy: -40 },
                Command::End,
            ],
            ..Default::default()
        };
        let bytes = encode(&d, &DstEncodeOptions::default()).unwrap();
        let text = |o: usize, len: usize| std::str::from_utf8(&bytes[o..o + len]).unwrap();
        // Field starts: LA 0, ST 20, CO 31, +X 38, then 9 bytes per
        // extent field and 10 per signed field.
        assert_eq!(text(20, 11), "ST:      3\r");
        assert_eq!(text(31, 7), "CO:  0\r");
        assert_eq!(text(38, 9), "+X:   30\r");
        assert_eq!(text(74, 10), "AX:+    0\r");
        assert_eq!(text(84, 10), "AY:+    0\r");
        assert_eq!(text(94, 10), "MX:+    0\r");
        assert_eq!(text(104, 10), "MY:+    0\r");
        assert_eq!(text(114, 10), "PD:******\r");
        assert_eq!(bytes[124], 0x1A);
        assert!(bytes[125..HEADER_LEN].iter().all(|&b| b == 0x20));
    }

    #[test]
    fn deviating_writer_forms_accepted() {
        // The corpus's one deviating writer zero-pads `MX:+   00`,
        // writes a negative-signed zero `AX:-    0`, and dashes for
        // `PD:`. All must parse.
        assert_eq!(parse_num("+    0"), Some(0));
        assert_eq!(parse_num("-    0"), Some(0));
        assert_eq!(parse_num("+   00"), Some(0));
        assert_eq!(parse_num("- 1234"), Some(-1234));
        assert_eq!(parse_num("  647"), Some(647));
        assert_eq!(parse_num("------"), None);
        assert_eq!(parse_num("******"), None);
        assert_eq!(parse_num("      "), None);
    }

    #[test]
    fn header_st_matches_documented_semantics() {
        // ST: = stitches + jumps + colour changes + 1 end record.
        let d = sample_design();
        let bytes = encode(&d, &DstEncodeOptions::default()).unwrap();
        let f = decode(&bytes).unwrap();
        let c = f.design.counts();
        assert_eq!(
            f.header.stitch_records.unwrap() as usize,
            c.stitches + c.jumps + c.color_changes + 1
        );
    }

    #[test]
    fn long_moves_split() {
        let d = Design {
            commands: vec![Command::Stitch { dx: 300, dy: -250 }, Command::End],
            ..Default::default()
        };
        let bytes = encode(&d, &DstEncodeOptions::default()).unwrap();
        let f = decode(&bytes).unwrap();
        // 300 → 121+121+58 (three records).
        assert_eq!(f.design.counts().stitches, 3);
        let p = f.design.positions();
        assert_eq!(p[p.len() - 2], (300, -250));
    }

    #[test]
    fn trim_becomes_documented_jump_run() {
        let d = Design {
            commands: vec![
                Command::Stitch { dx: 1, dy: 0 },
                Command::Trim { dx: 0, dy: 0 },
                Command::Stitch { dx: 1, dy: 0 },
                Command::End,
            ],
            ..Default::default()
        };
        let bytes = encode(&d, &DstEncodeOptions::default()).unwrap();
        let f = decode(&bytes).unwrap();
        let c = f.design.counts();
        assert_eq!(c.jumps, 3); // +2+2, −4−4, +2+2
        assert_eq!(c.stitches, 2);
        // The jump run is a positional no-op.
        let p = f.design.positions();
        assert_eq!(p[p.len() - 2], (2, 0));
    }

    #[test]
    fn digitiser_variant_stray_byte_tolerated() {
        let d = sample_design();
        let mut bytes = encode(&d, &DstEncodeOptions::default()).unwrap();
        bytes.push(0x00); // (size − 512) % 3 == 1 variant
        let f = decode(&bytes).unwrap();
        assert!(f.terminated);
        assert_eq!(f.trailing_bytes, 1);
        assert_eq!(f.design.commands, d.commands);
    }

    #[test]
    fn stop_unsupported() {
        let d = Design {
            commands: vec![Command::Stop, Command::End],
            ..Default::default()
        };
        assert!(matches!(
            encode(&d, &DstEncodeOptions::default()),
            Err(Error::Unsupported { .. })
        ));
    }

    #[test]
    fn truncated_header_rejected() {
        assert!(matches!(
            decode(&[0u8; 100]),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn bad_magic_rejected() {
        let bytes = vec![b'X'; 600];
        assert!(matches!(decode(&bytes), Err(Error::BadMagic { .. })));
    }

    #[test]
    fn header_only_parse_reads_a_tape_family_header() {
        // The corpus mapping documents that .dsb/.dsz carry DST's
        // 512-byte header verbatim over differently-encoded records.
        // decode_header must therefore work on exactly one header
        // with no decodable body behind it.
        let d = sample_design();
        let full = encode(
            &d,
            &DstEncodeOptions {
                label: "TAPEFAM".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let mut header_only = full[..HEADER_LEN].to_vec();
        // Splice in an undecodable record body of the sibling shape.
        header_only.extend_from_slice(&[0xA1, 0x65, 0x1F]);
        let h = decode_header(&header_only).unwrap();
        assert_eq!(h.label, "TAPEFAM");
        assert_eq!(h.color_changes, Some(d.counts().color_changes as u32));
        let e = d.extents();
        assert_eq!(h.pos_x, Some(e.max_x.max(0)));
        assert_eq!(h.neg_y, Some((-e.min_y).max(0)));
    }
}
