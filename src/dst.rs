//! Tajima DST — decode and encode.
//!
//! Layout per the workspace's staged documentation
//! (`docs/embroidery/dst/`): a 512-byte ASCII header (label, counts,
//! extents, vestigial tape-splicing fields, closed by `0x1A` at
//! offset 124), then 3-byte stitch records whose x/y deltas are
//! ternary-coded bit weights (±1, ±3, ±9, ±27, ±81 per axis, so
//! ±121 per record), and a `00 00 F3` end record.
//!
//! Details the staged documentation leaves unpinned (recorded here so
//! they are auditable): the exact space-vs-zero padding of numeric
//! header values, and the population convention for the vestigial
//! `AX/AY/MX/MY/PD` fields. This encoder right-aligns numbers with
//! space padding, writes `AX/AY` as the signed needle end point and
//! leaves `MX/MY/PD` blank; the decoder treats all of these leniently.

use crate::model::{Command, Design};
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
pub struct DstEncodeOptions {
    /// Design label for the `LA:` field (up to 16 ASCII characters).
    pub label: String,
    /// Express [`Command::Trim`] as the documented
    /// trim-by-consecutive-jumps convention: a deliberate no-op jump
    /// run (+2+2, −4−4, +2+2) that machines recognise as a trim
    /// request. When `false`, a trim is encoded as a single jump.
    pub trim_as_jump_run: bool,
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
    t.parse::<i32>().ok()
}

/// Decodes a DST file.
pub fn decode(data: &[u8]) -> Result<DstFile> {
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

    let mut commands = Vec::new();
    let mut terminated = false;
    let mut trailing = 0usize;
    let body = &data[HEADER_LEN..];
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
        },
        terminated,
        trailing_bytes: trailing,
    })
}

/// Decodes the x/y delta of one 3-byte record per the ternary bit
/// table.
fn decode_delta(b1: u8, b2: u8, b3: u8) -> (i32, i32) {
    let mut x = 0i32;
    let mut y = 0i32;
    // byte 1: y±1 (bits 7/6), y±9 (5/4), x∓9 (3/2 = −9/+9), x∓1 (1/0 = −1/+1)
    if b1 & 0x80 != 0 {
        y += 1;
    }
    if b1 & 0x40 != 0 {
        y -= 1;
    }
    if b1 & 0x20 != 0 {
        y += 9;
    }
    if b1 & 0x10 != 0 {
        y -= 9;
    }
    if b1 & 0x08 != 0 {
        x -= 9;
    }
    if b1 & 0x04 != 0 {
        x += 9;
    }
    if b1 & 0x02 != 0 {
        x -= 1;
    }
    if b1 & 0x01 != 0 {
        x += 1;
    }
    // byte 2: same pattern ×3
    if b2 & 0x80 != 0 {
        y += 3;
    }
    if b2 & 0x40 != 0 {
        y -= 3;
    }
    if b2 & 0x20 != 0 {
        y += 27;
    }
    if b2 & 0x10 != 0 {
        y -= 27;
    }
    if b2 & 0x08 != 0 {
        x -= 27;
    }
    if b2 & 0x04 != 0 {
        x += 27;
    }
    if b2 & 0x02 != 0 {
        x -= 3;
    }
    if b2 & 0x01 != 0 {
        x += 3;
    }
    // byte 3: y±81 (bits 5/4), x∓81 (3/2)
    if b3 & 0x20 != 0 {
        y += 81;
    }
    if b3 & 0x10 != 0 {
        y -= 81;
    }
    if b3 & 0x08 != 0 {
        x -= 81;
    }
    if b3 & 0x04 != 0 {
        x += 81;
    }
    (x, y)
}

/// Balanced-ternary digits (−1/0/+1) for weights 1, 3, 9, 27, 81.
/// Unique for every value in [−121, 121].
fn trits(mut v: i32) -> [i8; 5] {
    debug_assert!((-121..=121).contains(&v));
    let mut t = [0i8; 5];
    for d in &mut t {
        let r = v.rem_euclid(3);
        if r == 2 {
            *d = -1;
            v = (v + 1) / 3;
        } else {
            *d = r as i8;
            v = (v - r) / 3;
        }
    }
    t
}

/// Encodes one record's delta into the three record bytes (control
/// bits not included).
fn encode_delta(dx: i32, dy: i32) -> (u8, u8, u8) {
    let tx = trits(dx);
    let ty = trits(dy);
    let (mut b1, mut b2, mut b3) = (0u8, 0u8, 0u8);
    // (trit index, plus-bit, minus-bit) per byte, x axis
    for (ti, byte, plus, minus) in [
        (0, 1, 0x01u8, 0x02u8),
        (1, 2, 0x01, 0x02),
        (2, 1, 0x04, 0x08),
        (3, 2, 0x04, 0x08),
        (4, 3, 0x04, 0x08),
    ] {
        let bit = match tx[ti] {
            1 => plus,
            -1 => minus,
            _ => 0,
        };
        match byte {
            1 => b1 |= bit,
            2 => b2 |= bit,
            _ => b3 |= bit,
        }
    }
    // y axis
    for (ti, byte, plus, minus) in [
        (0, 1, 0x80u8, 0x40u8),
        (1, 2, 0x80, 0x40),
        (2, 1, 0x20, 0x10),
        (3, 2, 0x20, 0x10),
        (4, 3, 0x20, 0x10),
    ] {
        let bit = match ty[ti] {
            1 => plus,
            -1 => minus,
            _ => 0,
        };
        match byte {
            1 => b1 |= bit,
            2 => b2 |= bit,
            _ => b3 |= bit,
        }
    }
    (b1, b2, b3)
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
    let mut body = Vec::new();
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

    let e = design.extents();
    let end = design.positions().last().copied().unwrap_or((0, 0));
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(format!("LA:{:<16}\r", options.label).as_bytes());
    header.extend_from_slice(format!("ST:{records:7}\r").as_bytes());
    header.extend_from_slice(format!("CO:{color_changes:3}\r").as_bytes());
    header.extend_from_slice(format!("+X:{:5}\r", e.max_x.max(0)).as_bytes());
    header.extend_from_slice(format!("-X:{:5}\r", (-e.min_x).max(0)).as_bytes());
    header.extend_from_slice(format!("+Y:{:5}\r", e.max_y.max(0)).as_bytes());
    header.extend_from_slice(format!("-Y:{:5}\r", (-e.min_y).max(0)).as_bytes());
    header.extend_from_slice(format!("AX:{:+6}\r", end.0).as_bytes());
    header.extend_from_slice(format!("AY:{:+6}\r", end.1).as_bytes());
    header.extend_from_slice(format!("MX:{:6}\r", 0).as_bytes());
    header.extend_from_slice(format!("MY:{:6}\r", 0).as_bytes());
    header.extend_from_slice(format!("PD:{:6}\r", "").as_bytes());
    debug_assert_eq!(header.len(), 124);
    header.push(0x1A);
    header.resize(HEADER_LEN, b' ');

    let mut out = header;
    out.extend_from_slice(&body);
    Ok(out)
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
    fn wilcom_stray_byte_tolerated() {
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
}
