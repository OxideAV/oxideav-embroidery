//! Janome JEF — decode + encode.
//!
//! Per the workspace's staged documentation (`docs/embroidery/jef/`,
//! header mapped and validated against a real file): a little-endian
//! header (stitch-data offset at 0, format flag, date area, colour
//! count, record count, hoop code, extents, a colour table of Janome
//! thread codes at 0x74), then 2-byte stitch records with `0x80` as
//! the escape byte — `80 01` + delta pair = colour change, `80 02` +
//! delta pair = jump, `80 10` = end of design.
//!
//! **The JEF container family** (corpus-pinned): `.jef`, `.ptn`,
//! `.jef+` and `.jpx` share this container, distinguished by the
//! format flag at 0x04 — JEF and PTN carry 10, JEF+ and JPX carry 1.
//! A `.ptn` is a plain JEF (the corpus's Small `.ptn` is
//! byte-identical to its `.jef`). The flag-1 members carry the same
//! stitch vocabulary at their stitch offset (corpus-observed: the
//! walk ends on `80 10` exactly), but their colour-table layout past
//! the fixed header is not pinned — JPX does not even keep the
//! colour count at 0x18 — so flag-1 decode returns the stitch design
//! with an empty colour table and ignores any bytes after the end
//! marker (JEF+ carries a second, undocumented region there). The
//! date field at 0x08 is real: PTN/JPX/JEF+ writers stamp
//! `"20170330203029"`-style ASCII dates where plain JEF may leave it
//! zero-filled.
//!
//! One detail is derived here from the staged numbers rather than
//! stated outright by them, and is recorded as an auditable choice:
//! the header record count equals stitches + 2×jumps + 2×colour
//! changes + 1 (the only formula matching the validated sample). The
//! extents order `(|−X|, +Y, +X, |−Y|)` is corpus-pinned (checked
//! against sibling DST headers on three designs). The second
//! per-colour u32 array between the colour table and the stitch data
//! is **uniformly 13** in every corpus file (8 files, 3 designs,
//! 3–11 colours) with semantics unknown; it is preserved raw on
//! decode and 13-filled on encode. The hoop code at 0x20 is a small
//! enum tracking the design envelope — 0, 2 and 29 are the observed
//! values.

use crate::model::{Command, Design, Thread};
use crate::{Error, Result};

/// A decoded JEF file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JefFile {
    /// Format/version flag at 0x04 (observed: 10).
    pub format_flag: u32,
    /// Raw 16-byte date-stamp area at 0x08 (ASCII in some writers).
    pub date: [u8; 16],
    /// Header record count at 0x1C.
    pub record_count: u32,
    /// Hoop code at 0x20.
    pub hoop: u32,
    /// Raw extents group at 0x24 (order per module docs).
    pub extents: [i32; 4],
    /// Further extent sets (0x34..), `-1` when unused.
    pub extra_extents: [[i32; 4]; 4],
    /// Janome thread codes, one per colour.
    pub color_table: Vec<u32>,
    /// The undocumented second per-colour u32 array, raw.
    pub second_table: Vec<u32>,
    /// The stitch design.
    pub design: Design,
}

impl JefFile {
    /// The ASCII date stamp from the 0x08 area
    /// (`"20170330203029"`-style), when the writer populated it.
    /// PTN/JPX/JEF+ writers stamp it; plain JEF may zero-fill.
    pub fn date_string(&self) -> Option<&str> {
        let end = self
            .date
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.date.len());
        let s = std::str::from_utf8(&self.date[..end]).ok()?;
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then_some(s)
    }
}

/// Encoding options.
#[derive(Debug, Clone, Default)]
pub struct JefEncodeOptions {
    /// Hoop code for the 0x20 header field (enumeration not staged;
    /// the validated sample used 2).
    pub hoop: u32,
    /// Janome thread codes, one per colour block. When empty, codes
    /// are taken from `design.threads` palette indices, falling back
    /// to `1, 2, 3, …`.
    pub color_table: Vec<u32>,
}

const HEADER_FIXED: usize = 0x74;

/// Format flag of plain JEF (and PTN).
pub const FORMAT_FLAG_JEF: u32 = 10;
/// Format flag of the JEF+ / JPX family members.
pub const FORMAT_FLAG_PLUS: u32 = 1;

/// Returns true when `data` plausibly starts a JEF-family file: the
/// stitch offset points inside the file past the fixed header and
/// the format flag is one of the two documented family values.
pub fn probe(data: &[u8]) -> bool {
    if data.len() < HEADER_FIXED + 4 {
        return false;
    }
    let off = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let flag = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    (HEADER_FIXED..data.len()).contains(&off)
        && (flag == FORMAT_FLAG_JEF || flag == FORMAT_FLAG_PLUS)
}

/// Decodes a JEF file.
pub fn decode(data: &[u8]) -> Result<JefFile> {
    if data.len() < HEADER_FIXED {
        return Err(Error::UnexpectedEof {
            context: "JEF header",
        });
    }
    let u32le = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let i32le = |o: usize| u32le(o) as i32;
    let stitch_offset = u32le(0) as usize;
    let format_flag = u32le(4);
    let mut date = [0u8; 16];
    date.copy_from_slice(&data[0x08..0x18]);
    let colors = u32le(0x18) as usize;
    let record_count = u32le(0x1C);
    let hoop = u32le(0x20);
    let extents = [i32le(0x24), i32le(0x28), i32le(0x2C), i32le(0x30)];
    let mut extra_extents = [[0i32; 4]; 4];
    for (g, group) in extra_extents.iter_mut().enumerate() {
        for (k, v) in group.iter_mut().enumerate() {
            *v = i32le(0x34 + g * 16 + k * 4);
        }
    }
    let (color_table, second_table) = match format_flag {
        FORMAT_FLAG_JEF => {
            if colors > 0x100 {
                return Err(Error::invalid(format!("JEF colour count {colors}")));
            }
            if stitch_offset < HEADER_FIXED + 4 * colors || stitch_offset > data.len() {
                return Err(Error::invalid(format!(
                    "JEF stitch offset {stitch_offset} inconsistent with {colors} colours and {} bytes",
                    data.len()
                )));
            }
            let table: Vec<u32> = (0..colors).map(|i| u32le(HEADER_FIXED + 4 * i)).collect();
            let second_start = HEADER_FIXED + 4 * colors;
            let second: Vec<u32> = (0..(stitch_offset - second_start) / 4)
                .map(|i| u32le(second_start + 4 * i))
                .collect();
            (table, second)
        }
        FORMAT_FLAG_PLUS => {
            // JEF+/JPX: same stitch vocabulary at the stitch offset,
            // but the colour-table layout is not pinned by the
            // staged docs (JPX does not keep the count at 0x18), so
            // no colour data is surfaced.
            if stitch_offset > data.len() {
                return Err(Error::invalid(format!(
                    "JEF+ stitch offset {stitch_offset} beyond {} bytes",
                    data.len()
                )));
            }
            (Vec::new(), Vec::new())
        }
        other => {
            return Err(Error::invalid(format!(
                "JEF format flag {other} not documented"
            )));
        }
    };

    let mut commands = Vec::new();
    let body = &data[stitch_offset..];
    let mut i = 0usize;
    let mut terminated = false;
    while i + 2 <= body.len() {
        let b0 = body[i];
        if b0 == 0x80 {
            let code = body[i + 1];
            i += 2;
            match code {
                0x01 | 0x02 => {
                    if i + 2 > body.len() {
                        return Err(Error::UnexpectedEof {
                            context: "JEF control delta",
                        });
                    }
                    let dx = body[i] as i8 as i32;
                    let dy = body[i + 1] as i8 as i32;
                    i += 2;
                    if code == 0x01 {
                        commands.push(Command::ColorChange {
                            dx,
                            dy,
                            index: None,
                        });
                    } else {
                        commands.push(Command::Jump { dx, dy });
                    }
                }
                0x10 => {
                    commands.push(Command::End);
                    terminated = true;
                    break;
                }
                other => {
                    return Err(Error::invalid(format!(
                        "JEF escape code 0x{other:02x} not documented"
                    )));
                }
            }
        } else {
            let dx = b0 as i8 as i32;
            let dy = body[i + 1] as i8 as i32;
            commands.push(Command::Stitch { dx, dy });
            i += 2;
        }
    }
    if !terminated && i < body.len() {
        return Err(Error::UnexpectedEof {
            context: "JEF stitch record",
        });
    }

    let threads = color_table
        .iter()
        .map(|&code| Thread {
            palette_index: Some(code.min(u16::MAX as u32) as u16),
            ..Default::default()
        })
        .collect();

    Ok(JefFile {
        format_flag,
        date,
        record_count,
        hoop,
        extents,
        extra_extents,
        color_table,
        second_table,
        design: Design {
            commands,
            threads,
            label: None,
        },
    })
}

fn push_move(out: &mut Vec<u8>, mut dx: i32, mut dy: i32, code: Option<u8>) -> usize {
    let mut records = 0usize;
    loop {
        let sx = dx.clamp(-127, 127);
        let sy = dy.clamp(-127, 127);
        if let Some(c) = code {
            out.push(0x80);
            out.push(c);
            records += 1;
        }
        out.push((sx as i8) as u8);
        out.push((sy as i8) as u8);
        records += 1;
        dx -= sx;
        dy -= sy;
        if dx == 0 && dy == 0 {
            return records;
        }
    }
}

/// Encodes a design as a JEF file. [`Command::Trim`] is expressed as
/// a jump; [`Command::Stop`] is unsupported.
pub fn encode(design: &Design, options: &JefEncodeOptions) -> Result<Vec<u8>> {
    let blocks = design.color_block_count().max(1);
    let color_table: Vec<u32> = if !options.color_table.is_empty() {
        options.color_table.clone()
    } else if design.threads.len() >= blocks {
        design
            .threads
            .iter()
            .take(blocks)
            .enumerate()
            .map(|(i, t)| t.palette_index.map(u32::from).unwrap_or(i as u32 + 1))
            .collect()
    } else {
        (1..=blocks as u32).collect()
    };
    if color_table.len() != blocks {
        return Err(Error::invalid(format!(
            "colour table has {} entries for {} colour blocks",
            color_table.len(),
            blocks
        )));
    }
    // Implementation cap (±10 m) bounding the record-splitting loop;
    // JEF's own extents fields are s32 and impose no practical limit.
    let pre = design.extents();
    if pre.min_x < -1_000_000
        || pre.max_x > 1_000_000
        || pre.min_y < -1_000_000
        || pre.max_y > 1_000_000
    {
        return Err(Error::OutOfRange {
            field: "JEF extents (implementation cap)",
        });
    }

    let mut body = Vec::new();
    let mut records = 0usize;
    for c in &design.commands {
        match *c {
            Command::Stitch { dx, dy } => records += push_move(&mut body, dx, dy, None),
            Command::Jump { dx, dy } | Command::Trim { dx, dy } => {
                records += push_move(&mut body, dx, dy, Some(0x02))
            }
            Command::ColorChange { dx, dy, .. } => {
                records += push_move(&mut body, dx, dy, Some(0x01))
            }
            Command::Stop => {
                return Err(Error::Unsupported {
                    what: "JEF has no stop record",
                });
            }
            Command::End => break,
        }
    }
    body.extend_from_slice(&[0x80, 0x10]);
    records += 1;

    let e = design.extents();
    let stitch_offset = (HEADER_FIXED + 8 * blocks) as u32;
    let mut out = Vec::with_capacity(stitch_offset as usize + body.len());
    out.extend_from_slice(&stitch_offset.to_le_bytes());
    out.extend_from_slice(&10u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&(blocks as u32).to_le_bytes());
    out.extend_from_slice(&(records as u32).to_le_bytes());
    out.extend_from_slice(&options.hoop.to_le_bytes());
    // Extents order per module docs: (|−X|, +Y, +X, |−Y|).
    out.extend_from_slice(&((-e.min_x).max(0)).to_le_bytes());
    out.extend_from_slice(&e.max_y.max(0).to_le_bytes());
    out.extend_from_slice(&e.max_x.max(0).to_le_bytes());
    out.extend_from_slice(&((-e.min_y).max(0)).to_le_bytes());
    for _ in 0..16 {
        out.extend_from_slice(&(-1i32).to_le_bytes());
    }
    debug_assert_eq!(out.len(), HEADER_FIXED);
    for code in &color_table {
        out.extend_from_slice(&code.to_le_bytes());
    }
    // The second per-colour array is uniformly 13 across the whole
    // corpus (semantics unknown).
    for _ in 0..blocks {
        out.extend_from_slice(&13u32.to_le_bytes());
    }
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Design};

    fn sample() -> Design {
        Design {
            commands: vec![
                Command::Jump { dx: 20, dy: 20 },
                Command::Stitch { dx: 9, dy: 0 },
                Command::Stitch { dx: 0, dy: 9 },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::Stitch { dx: -9, dy: -9 },
                Command::End,
            ],
            ..Default::default()
        }
    }

    #[test]
    fn roundtrip() {
        let d = sample();
        let bytes = encode(
            &d,
            &JefEncodeOptions {
                hoop: 2,
                color_table: vec![31, 24],
            },
        )
        .unwrap();
        assert!(probe(&bytes));
        let f = decode(&bytes).unwrap();
        assert_eq!(f.format_flag, 10);
        assert_eq!(f.hoop, 2);
        assert_eq!(f.color_table, vec![31, 24]);
        assert_eq!(f.second_table, vec![13, 13]);
        assert_eq!(f.design.commands, d.commands);
        assert_eq!(f.extra_extents, [[-1; 4]; 4]);
    }

    #[test]
    fn record_count_formula() {
        // Derived from the validated sample: stitches + 2×jumps +
        // 2×colour changes + 1 end record.
        let d = sample();
        let bytes = encode(&d, &JefEncodeOptions::default()).unwrap();
        let f = decode(&bytes).unwrap();
        let c = f.design.counts();
        assert_eq!(
            f.record_count as usize,
            c.stitches + 2 * c.jumps + 2 * c.color_changes + 1
        );
    }

    #[test]
    fn stitch_offset_documented_shape() {
        // 0x74 + 8 bytes per colour (sample: 204 at 11 colours).
        assert_eq!(HEADER_FIXED + 8 * 11, 204);
        let d = sample();
        let bytes = encode(&d, &JefEncodeOptions::default()).unwrap();
        let off = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(off as usize, HEADER_FIXED + 8 * 2);
    }

    #[test]
    fn extents_order_matches_validated_sample_convention() {
        // Sample header: (|−X|, +Y, +X, |−Y|) = (648, 907, 647, 908)
        // for extents X ∈ [−648, 647], Y ∈ [−908, 907].
        let d = Design {
            commands: vec![
                Command::Stitch { dx: -64, dy: -90 },
                Command::Stitch { dx: 127, dy: 180 },
                Command::End,
            ],
            ..Default::default()
        };
        let bytes = encode(&d, &JefEncodeOptions::default()).unwrap();
        let f = decode(&bytes).unwrap();
        assert_eq!(f.extents, [64, 90, 63, 90]);
    }

    #[test]
    fn family_flag_one_decodes_stitches_without_colors() {
        // JEF+/JPX: format flag 1, same stitch vocabulary, colour
        // layout unpinned → empty tables.
        let d = sample();
        let mut bytes = encode(&d, &JefEncodeOptions::default()).unwrap();
        bytes[4..8].copy_from_slice(&FORMAT_FLAG_PLUS.to_le_bytes());
        assert!(probe(&bytes));
        let f = decode(&bytes).unwrap();
        assert_eq!(f.format_flag, FORMAT_FLAG_PLUS);
        assert!(f.color_table.is_empty());
        assert!(f.second_table.is_empty());
        assert_eq!(f.design.commands, d.commands);
        assert!(f.design.threads.is_empty());
    }

    #[test]
    fn undocumented_flag_rejected() {
        let d = sample();
        let mut bytes = encode(&d, &JefEncodeOptions::default()).unwrap();
        bytes[4..8].copy_from_slice(&7u32.to_le_bytes());
        assert!(!probe(&bytes));
        assert!(matches!(decode(&bytes), Err(Error::Invalid { .. })));
    }

    #[test]
    fn date_string_helper() {
        let d = sample();
        let bytes = encode(&d, &JefEncodeOptions::default()).unwrap();
        let f = decode(&bytes).unwrap();
        assert_eq!(f.date_string(), None); // zero-filled on encode
        let mut stamped = f.clone();
        stamped.date[..14].copy_from_slice(b"20170330203029");
        assert_eq!(stamped.date_string(), Some("20170330203029"));
    }

    #[test]
    fn undocumented_escape_rejected() {
        let d = sample();
        let mut bytes = encode(&d, &JefEncodeOptions::default()).unwrap();
        let off = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        bytes[off] = 0x80;
        bytes[off + 1] = 0x42;
        assert!(matches!(decode(&bytes), Err(Error::Invalid { .. })));
    }

    #[test]
    fn truncated_rejected() {
        assert!(matches!(
            decode(&[0u8; 50]),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}
