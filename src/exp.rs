//! Melco EXP — headerless stitch list, decode + encode, plus the
//! big-endian `.inf` colour companion.
//!
//! Per the workspace's staged documentation (`docs/embroidery/exp/`,
//! Grade-A table validated against real files): design data starts
//! at byte 0; a normal stitch is 2 bytes (`dx`, `dy`, signed 8-bit);
//! control records are 4 bytes — `0x80`, a code byte, two payload
//! bytes. No colours, no extents, no end marker are stored.
//!
//! # Control-code vocabulary and its evidence
//!
//! | Code | Decoded as | Evidence |
//! | ---- | ---------- | -------- |
//! | `80 01` | colour change, payload = move | corpus (43 records over 9 files, matching the sibling DST `CO:` fields) |
//! | `80 04` | jump, payload = move | corpus (938 records over 9 files; record-identical to the HUS siblings) |
//! | `80 02` | jump, payload = move | the Grade-A table's worked example only; never seen in a staged file, so flagged |
//! | other odd | colour change, payload = move | the documented general rule only (both staged descriptions agree); flagged |
//! | other even (incl. `80 80`) | **skipped — no command, no movement** | none: the staged descriptions disagree (jump / stop / end) and no staged file carries one |
//!
//! "Flagged" and "skipped" records are recorded in
//! [`Design::diagnostics`]; [`decode_with`] under strict options
//! rejects them with [`Error::UnknownControl`]. An unevidenced even
//! code is deliberately **not** turned into needle movement: if its
//! payload is not a move, treating it as one shifts every later
//! stitch of the design.
//!
//! The staged corpus cannot say what an EXP trim looks like: its
//! EXP files are record-identical to their HUS siblings *after
//! dropping the HUS trim records*, i.e. that producer writes no trim
//! record into EXP at all. The encoder therefore still expresses
//! [`Command::Trim`] as an `80 04` jump. There is **no end-of-design
//! escape** in the corpus — files simply end — so decode runs to
//! end of input.
//!
//! EXP ships alongside a `.inf` colour file (159-byte five-colour
//! sample mapped field-by-field in the staged notes).

use crate::model::{Command, Design, Diagnostics, Thread, WarningAction};
use crate::{DecodeOptions, Error, Result};

/// Maximum per-axis delta of one record.
pub const MAX_DELTA: i32 = 127;

/// The control escape byte (an X delta of −128 never occurs).
const ESCAPE: u8 = 0x80;
/// Colour change (corpus-pinned).
const CODE_COLOR_CHANGE: u8 = 0x01;
/// Jump as the Grade-A worked example writes it (never observed).
const CODE_JUMP_TABLE_EXAMPLE: u8 = 0x02;
/// Jump (corpus-pinned).
const CODE_JUMP: u8 = 0x04;

/// Decodes an EXP stitch list (the whole file), leniently: control
/// codes without file evidence are flagged or skipped as the module
/// docs tabulate, and listed in [`Design::diagnostics`].
pub fn decode(data: &[u8]) -> Result<Design> {
    // Every record is at least two bytes, so the command count is
    // bounded by half the input: one exact allocation.
    let mut commands = Vec::with_capacity(data.len() / 2 + 1);
    let mut diagnostics = Diagnostics::default();
    let mut i = 0usize;
    while i < data.len() {
        let b0 = data[i];
        if b0 != ESCAPE {
            let Some(&b1) = data.get(i + 1) else {
                return Err(Error::UnexpectedEof {
                    context: "EXP stitch record",
                });
            };
            commands.push(Command::Stitch {
                dx: b0 as i8 as i32,
                dy: b1 as i8 as i32,
            });
            i += 2;
            continue;
        }
        let Some(&[_, code, px, py]) = data.get(i..i + 4).and_then(|r| r.as_array::<4>()) else {
            return Err(Error::UnexpectedEof {
                context: "EXP control record",
            });
        };
        let (dx, dy) = (px as i8 as i32, py as i8 as i32);
        match code {
            CODE_JUMP => commands.push(Command::Jump { dx, dy }),
            CODE_COLOR_CHANGE => commands.push(Command::ColorChange {
                dx,
                dy,
                index: None,
            }),
            CODE_JUMP_TABLE_EXAMPLE => {
                diagnostics.push("EXP", i, code, [px, py], WarningAction::DecodedByRule);
                commands.push(Command::Jump { dx, dy });
            }
            odd if odd % 2 == 1 => {
                diagnostics.push("EXP", i, code, [px, py], WarningAction::DecodedByRule);
                commands.push(Command::ColorChange {
                    dx,
                    dy,
                    index: None,
                });
            }
            _ => diagnostics.push("EXP", i, code, [px, py], WarningAction::Skipped),
        }
        i += 4;
    }
    commands.push(Command::End);
    Ok(Design {
        commands,
        diagnostics,
        ..Default::default()
    })
}

/// [`decode`] under explicit options: strict options reject the
/// first control code that is not corpus-pinned.
pub fn decode_with(data: &[u8], options: &DecodeOptions) -> Result<Design> {
    let design = decode(data)?;
    options.check(&design)?;
    Ok(design)
}

fn push_move(out: &mut Vec<u8>, mut dx: i32, mut dy: i32, control: Option<u8>) {
    loop {
        let sx = dx.clamp(-MAX_DELTA, MAX_DELTA);
        let sy = dy.clamp(-MAX_DELTA, MAX_DELTA);
        if let Some(code) = control {
            out.push(ESCAPE);
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
/// expressed as a jump (no staged file shows an EXP trim record —
/// see the module docs); [`Command::Stop`] is unsupported.
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
    let mut out = Vec::with_capacity(design.commands.len() * 2);
    for c in &design.commands {
        match *c {
            Command::Stitch { dx, dy } => push_move(&mut out, dx, dy, None),
            Command::Jump { dx, dy } | Command::Trim { dx, dy } => {
                // The corpus-pinned jump escape is `80 04` (`80 02`
                // never occurs in real files).
                push_move(&mut out, dx, dy, Some(CODE_JUMP))
            }
            Command::ColorChange { dx, dy, .. } => {
                push_move(&mut out, dx, dy, Some(CODE_COLOR_CHANGE))
            }
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
    fn corpus_pinned_codes_decode_without_warnings() {
        let d = decode(&[0x80, 0x01, 0, 0, 0x80, 0x04, 1, 1, 5, 5]).unwrap();
        assert!(matches!(d.commands[0], Command::ColorChange { .. }));
        assert!(matches!(d.commands[1], Command::Jump { dx: 1, dy: 1 }));
        assert!(d.diagnostics.is_empty());
        assert!(decode_with(&[0x80, 0x04, 1, 1], &DecodeOptions::strict()).is_ok());
    }

    #[test]
    fn rule_only_codes_are_decoded_and_flagged() {
        // 80 02 (the table's worked example) and an unobserved odd
        // code follow the documented rule, but are reported.
        let d = decode(&[0x80, 0x02, 3, 4, 0x80, 0x07, 0, 0]).unwrap();
        assert_eq!(d.commands[0], Command::Jump { dx: 3, dy: 4 });
        assert!(matches!(d.commands[1], Command::ColorChange { .. }));
        let w = d.diagnostics.warnings();
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].code, w[0].offset, w[0].payload), (0x02, 0, [3, 4]));
        assert_eq!(w[0].action, WarningAction::DecodedByRule);
        assert_eq!((w[1].code, w[1].offset), (0x07, 4));
    }

    #[test]
    fn unevidenced_even_code_never_moves_the_needle() {
        // 80 80 07 00 between two stitches: lenient decode consumes
        // the record without movement and reports it; strict decode
        // returns the typed error.
        let bytes = [10u8, 0, 0x80, 0x80, 0x07, 0x00, 0, 10];
        let d = decode(&bytes).unwrap();
        assert_eq!(
            d.commands,
            vec![
                Command::Stitch { dx: 10, dy: 0 },
                Command::Stitch { dx: 0, dy: 10 },
                Command::End
            ]
        );
        assert_eq!(d.positions().last(), Some(&(10, 10)));
        let w = d.diagnostics.warnings();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].format, "EXP");
        assert_eq!((w[0].code, w[0].offset, w[0].payload), (0x80, 2, [7, 0]));
        assert_eq!(w[0].action, WarningAction::Skipped);
        assert!(matches!(
            decode_with(&bytes, &DecodeOptions::strict()),
            Err(Error::UnknownControl {
                format: "EXP",
                code: 0x80,
                offset: 2
            })
        ));
        // Every other even code outside {02, 04} behaves the same.
        for code in (0u8..=0xFE).step_by(2).filter(|c| ![2, 4].contains(c)) {
            let d = decode(&[0x80, code, 9, 9]).unwrap();
            assert_eq!(d.commands, vec![Command::End], "code {code:#04x}");
            assert_eq!(d.diagnostics.total(), 1);
        }
    }

    #[test]
    fn warning_list_is_bounded_but_counted() {
        let bytes: Vec<u8> = [0x80u8, 0x80, 7, 0].repeat(1000);
        let d = decode(&bytes).unwrap();
        assert_eq!(d.diagnostics.total(), 1000);
        assert_eq!(d.diagnostics.warnings().len(), Diagnostics::MAX_RECORDED);
        assert_eq!(d.extents(), crate::Extents::default());
    }

    #[test]
    fn encoder_output_is_strict_clean() {
        let d = Design {
            commands: vec![
                Command::Jump { dx: 300, dy: -10 },
                Command::Trim { dx: 0, dy: 0 },
                Command::ColorChange {
                    dx: 1,
                    dy: 1,
                    index: None,
                },
                Command::Stitch { dx: -128, dy: 127 },
                Command::End,
            ],
            ..Default::default()
        };
        let bytes = encode(&d).unwrap();
        let got = decode_with(&bytes, &DecodeOptions::strict()).unwrap();
        assert_eq!(got.positions().last(), d.positions().last());
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
