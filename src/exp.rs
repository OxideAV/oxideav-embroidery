//! Melco EXP — headerless stitch list, decode + encode, plus the
//! big-endian `.inf` colour companion.
//!
//! Per the workspace's staged documentation (`docs/embroidery/exp/`,
//! Grade-A table validated against real files): design data starts
//! at byte 0; a normal stitch is 2 bytes (`dx`, `dy`, signed 8-bit);
//! control records are 4 bytes — `0x80` then a code byte (odd =
//! colour change, even = jump) then the 2-byte delta. No colours, no
//! extents, no end marker are stored: the corpus pins the practical
//! escape vocabulary to `80 01` (colour change) and `80 04` (jump)
//! only, with **no end-of-design escape** — files simply end — so
//! decode runs to end of input and this encoder writes `80 04` for
//! jumps.
//!
//! EXP ships alongside a `.inf` colour file (159-byte five-colour
//! sample mapped field-by-field in
//! `docs/embroidery/provenance/06-phc-offset-rule.md`).

use crate::model::{Command, Design, Thread};
use crate::{Error, Result};

/// Maximum per-axis delta of one record.
pub const MAX_DELTA: i32 = 127;

/// Decodes an EXP stitch list (the whole file).
pub fn decode(data: &[u8]) -> Result<Design> {
    let mut commands = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let b0 = data[i];
        if b0 == 0x80 {
            if i + 4 > data.len() {
                return Err(Error::UnexpectedEof {
                    context: "EXP control record",
                });
            }
            let code = data[i + 1];
            let dx = data[i + 2] as i8 as i32;
            let dy = data[i + 3] as i8 as i32;
            if code % 2 == 1 {
                commands.push(Command::ColorChange {
                    dx,
                    dy,
                    index: None,
                });
            } else {
                commands.push(Command::Jump { dx, dy });
            }
            i += 4;
        } else {
            if i + 2 > data.len() {
                return Err(Error::UnexpectedEof {
                    context: "EXP stitch record",
                });
            }
            let dx = b0 as i8 as i32;
            let dy = data[i + 1] as i8 as i32;
            commands.push(Command::Stitch { dx, dy });
            i += 2;
        }
    }
    commands.push(Command::End);
    Ok(Design {
        commands,
        threads: Vec::new(),
        label: None,
    })
}

fn push_move(out: &mut Vec<u8>, mut dx: i32, mut dy: i32, control: Option<u8>) {
    loop {
        let sx = dx.clamp(-MAX_DELTA, MAX_DELTA);
        let sy = dy.clamp(-MAX_DELTA, MAX_DELTA);
        if let Some(code) = control {
            out.push(0x80);
            out.push(code);
        }
        out.push((sx as i8) as u8);
        out.push((sy as i8) as u8);
        dx -= sx;
        dy -= sy;
        if dx == 0 && dy == 0 {
            return;
        }
    }
}

/// Encodes a design as an EXP stitch list. [`Command::Trim`] is
/// expressed as a jump (EXP has no confirmed trim escape);
/// [`Command::Stop`] is unsupported.
pub fn encode(design: &Design) -> Result<Vec<u8>> {
    // EXP stores no extents, so there is no format limit to enforce;
    // this implementation cap (±10 m) keeps hostile designs from
    // driving the record-splitting loop unboundedly.
    let pre = design.extents();
    if pre.min_x < -1_000_000
        || pre.max_x > 1_000_000
        || pre.min_y < -1_000_000
        || pre.max_y > 1_000_000
    {
        return Err(Error::OutOfRange {
            field: "EXP extents (implementation cap)",
        });
    }
    let mut out = Vec::new();
    for c in &design.commands {
        match *c {
            Command::Stitch { dx, dy } => push_move(&mut out, dx, dy, None),
            Command::Jump { dx, dy } | Command::Trim { dx, dy } => {
                // The corpus-pinned jump escape is `80 04` (`80 02`
                // never occurs in real files).
                push_move(&mut out, dx, dy, Some(0x04))
            }
            Command::ColorChange { dx, dy, .. } => push_move(&mut out, dx, dy, Some(0x01)),
            Command::Stop => {
                return Err(Error::Unsupported {
                    what: "EXP has no stop record",
                });
            }
            Command::End => break,
        }
    }
    Ok(out)
}

/// Decodes an `.inf` colour companion file (big-endian).
pub fn decode_inf(data: &[u8]) -> Result<Vec<Thread>> {
    let u32be = |o: usize| -> Result<u32> {
        data.get(o..o + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or(Error::UnexpectedEof {
                context: "INF header",
            })
    };
    let _version = u32be(0)?;
    let _u1 = u32be(4)?;
    let record_area = u32be(8)? as usize;
    let count = u32be(12)? as usize;
    // Two record-area conventions occur in the corpus: the length of
    // the record bytes from offset 16, and the same plus the 8 bytes
    // of the record-area and count fields themselves (i.e. counted
    // from offset 8). Accept both.
    if 16 + record_area != data.len() && 8 + record_area != data.len() {
        return Err(Error::UnexpectedEof {
            context: "INF record area",
        });
    }
    // Every record is at least 10 bytes, so the declared colour
    // count is bounded by the record bytes actually present — reject
    // inconsistent headers instead of pre-allocating from an
    // untrusted count.
    if count > (data.len() - 16) / 10 {
        return Err(Error::invalid(format!(
            "INF colour count {count} impossible for a {}-byte record area",
            data.len() - 16
        )));
    }
    let mut threads = Vec::with_capacity(count);
    let mut o = 16usize;
    for _ in 0..count {
        let len = data
            .get(o..o + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]) as usize)
            .ok_or(Error::UnexpectedEof {
                context: "INF record",
            })?;
        let rec = data.get(o..o + len).ok_or(Error::UnexpectedEof {
            context: "INF record",
        })?;
        if len < 10 {
            return Err(Error::invalid("INF record too short"));
        }
        let index = u16::from_be_bytes([rec[2], rec[3]]);
        let rgb = [rec[4], rec[5], rec[6]];
        // rec[7..9] repeats the index; then two NUL-terminated strings.
        let mut strings = rec[9..].split(|&b| b == 0);
        let catalog = strings
            .next()
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .filter(|s| !s.is_empty());
        let name = strings
            .next()
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .filter(|s| !s.is_empty());
        threads.push(Thread {
            palette_index: Some(index),
            rgb: Some(rgb),
            catalog,
            name,
        });
        o += len;
    }
    Ok(threads)
}

/// Encodes an `.inf` colour companion for the given threads.
/// Thread indices are written 1-based in order; missing RGB encodes
/// as black; a missing catalogue tag falls back to the documented
/// literal `RGB` + `RGB(r,g,b)` description.
pub fn encode_inf(threads: &[Thread]) -> Result<Vec<u8>> {
    let mut records = Vec::new();
    for (i, t) in threads.iter().enumerate() {
        let rgb = t.rgb.unwrap_or([0, 0, 0]);
        let catalog = t.catalog.clone().unwrap_or_else(|| "RGB".to_string());
        let name = t
            .name
            .clone()
            .unwrap_or_else(|| format!("RGB({},{},{})", rgb[0], rgb[1], rgb[2]));
        let index = t.palette_index.unwrap_or(i as u16 + 1);
        let len = 9 + catalog.len() + 1 + name.len() + 1;
        records.extend_from_slice(&(len as u16).to_be_bytes());
        records.extend_from_slice(&index.to_be_bytes());
        records.extend_from_slice(&rgb);
        records.extend_from_slice(&index.to_be_bytes());
        records.extend_from_slice(catalog.as_bytes());
        records.push(0);
        records.extend_from_slice(name.as_bytes());
        records.push(0);
    }
    let mut out = Vec::with_capacity(16 + records.len());
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&8u32.to_be_bytes());
    out.extend_from_slice(&(records.len() as u32).to_be_bytes());
    out.extend_from_slice(&(threads.len() as u32).to_be_bytes());
    out.extend_from_slice(&records);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Design};

    // Worked examples from the staged Grade-A table.
    #[test]
    fn worked_examples() {
        // 00 7F → x+0, y+127 ; 7F 00 → x+127, y+0
        let d = decode(&[0x00, 0x7F, 0x7F, 0x00]).unwrap();
        assert_eq!(
            d.commands,
            vec![
                Command::Stitch { dx: 0, dy: 127 },
                Command::Stitch { dx: 127, dy: 0 },
                Command::End
            ]
        );
        // 80 02 7F 00 → jump x+127 ; 80 01 00 81 → colour change y−127
        let d = decode(&[0x80, 0x02, 0x7F, 0x00, 0x80, 0x01, 0x00, 0x81]).unwrap();
        assert_eq!(
            d.commands,
            vec![
                Command::Jump { dx: 127, dy: 0 },
                Command::ColorChange {
                    dx: 0,
                    dy: -127,
                    index: None
                },
                Command::End
            ]
        );
    }

    #[test]
    fn odd_codes_are_color_changes_even_are_jumps() {
        let d = decode(&[0x80, 0x07, 0, 0, 0x80, 0x04, 1, 1]).unwrap();
        assert!(matches!(d.commands[0], Command::ColorChange { .. }));
        assert!(matches!(d.commands[1], Command::Jump { dx: 1, dy: 1 }));
    }

    #[test]
    fn roundtrip() {
        let d = Design {
            commands: vec![
                Command::Jump { dx: 300, dy: -10 },
                Command::Stitch { dx: 5, dy: -5 },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::Stitch { dx: -127, dy: 127 },
                Command::End,
            ],
            ..Default::default()
        };
        let bytes = encode(&d).unwrap();
        let got = decode(&bytes).unwrap();
        // The oversized jump splits into three.
        assert_eq!(got.counts().jumps, 3);
        assert_eq!(got.counts().stitches, 2);
        assert_eq!(got.counts().color_changes, 1);
        assert_eq!(got.positions().last(), d.positions().last());
    }

    #[test]
    fn trim_encodes_as_jump() {
        let d = Design {
            commands: vec![Command::Trim { dx: 3, dy: 4 }, Command::End],
            ..Default::default()
        };
        let bytes = encode(&d).unwrap();
        // Jumps use the corpus-pinned `80 04` escape.
        assert_eq!(bytes, vec![0x80, 0x04, 3, 4]);
    }

    #[test]
    fn inf_impossible_color_count_rejected() {
        // A declared count far larger than the record area could hold
        // must fail cleanly (no allocation from the untrusted count).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(&4u32.to_be_bytes()); // record area: 4 bytes
        bytes.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // count
        bytes.extend_from_slice(&[0u8; 4]);
        assert!(matches!(decode_inf(&bytes), Err(Error::Invalid { .. })));
    }

    #[test]
    fn truncated_control_errors() {
        assert!(matches!(
            decode(&[0x80, 0x02, 0x01]),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn inf_roundtrip() {
        let threads = vec![
            crate::model::Thread {
                palette_index: Some(1),
                rgb: Some([138, 182, 188]),
                catalog: None,
                name: None,
            },
            crate::model::Thread {
                palette_index: Some(2),
                rgb: Some([10, 20, 30]),
                catalog: Some("IR5".into()),
                name: Some("Steel".into()),
            },
        ];
        let bytes = encode_inf(&threads).unwrap();
        // Documented layout: BE version 1 at 0, record-area length at
        // 8, colour count at 12.
        assert_eq!(&bytes[..4], &1u32.to_be_bytes());
        assert_eq!(
            u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            2
        );
        let got = decode_inf(&bytes).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].rgb, Some([138, 182, 188]));
        assert_eq!(got[0].catalog.as_deref(), Some("RGB"));
        assert_eq!(got[0].name.as_deref(), Some("RGB(138,182,188)"));
        assert_eq!(got[1].catalog.as_deref(), Some("IR5"));
        assert_eq!(got[1].name.as_deref(), Some("Steel"));
        // Documented sample: a 30-byte record for "RGB(138,182,188)".
        assert_eq!(u16::from_be_bytes([bytes[16], bytes[17]]), 30);
    }
}
