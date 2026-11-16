//! The EXP trim record, pinned by purpose-made fixtures.
//!
//! `tests/fixtures/exp_trim/` holds one design written as EXP and as
//! PEC by two unrelated producers (provenance in the README beside
//! the files). PEC carries an explicit trim flag, so the PEC
//! renderings say where the trims of the design are; the EXP files
//! carry an `80 80 07 00` record at exactly those needle positions.

use oxideav_embroidery::{Command, DecodeOptions, Design, exp, pec};

const EXP_A: &[u8] = include_bytes!("fixtures/exp_trim/EXP_trim_A.exp");
const EXP_B: &[u8] = include_bytes!("fixtures/exp_trim/EXP_trim_B.exp");
const PEC_A: &[u8] = include_bytes!("fixtures/exp_trim/EXP_trim_A.pec");
const PEC_B: &[u8] = include_bytes!("fixtures/exp_trim/EXP_trim_B.pec");

/// The design: ten stitch blocks 250 units apart on the X axis.
const BLOCKS: i32 = 10;
const PITCH: i32 = 250;
const FIRST_BLOCK_X: i32 = -1225;
/// Where the last needle penetration of the design falls.
const LAST_PENETRATION_X: i32 = 681;

/// Needle penetrations: the position after every stitch command.
fn penetrations(d: &Design) -> Vec<(i32, i32)> {
    d.commands
        .iter()
        .zip(d.positions())
        .filter(|(c, _)| matches!(c, Command::Stitch { .. }))
        .map(|(_, p)| p)
        .collect()
}

/// Where each trim cuts: the needle position *before* the command's
/// own displacement (PEC folds the travel into the trim record; the
/// EXP record carries none).
fn trim_sites(d: &Design) -> Vec<(i32, i32)> {
    let positions = d.positions();
    d.commands
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c, Command::Trim { .. }))
        .map(|(i, _)| if i == 0 { (0, 0) } else { positions[i - 1] })
        .collect()
}

fn shifted(points: &[(i32, i32)], dx: i32) -> Vec<(i32, i32)> {
    points.iter().map(|&(x, y)| (x + dx, y)).collect()
}

/// Payloads of every `80 80` record, from a raw walk of the records.
fn raw_trim_payloads(data: &[u8]) -> Vec<[u8; 2]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0x80 {
            if data[i + 1] == 0x80 {
                out.push([data[i + 2], data[i + 3]]);
            }
            i += 4;
        } else {
            i += 2;
        }
    }
    out
}

#[test]
fn every_trim_record_carries_the_same_payload() {
    assert_eq!(raw_trim_payloads(EXP_A), vec![[0x07, 0x00]; 11]);
    assert_eq!(raw_trim_payloads(EXP_B), vec![[0x07, 0x00]; 10]);
}

#[test]
fn trim_fixtures_decode_strict_clean() {
    let strict = DecodeOptions::strict();
    for (name, data, trims, jumps) in [("A", EXP_A, 11, 39), ("B", EXP_B, 10, 31)] {
        let d = exp::decode_with(data, &strict).unwrap_or_else(|e| panic!("EXP {name}: {e:?}"));
        assert!(d.diagnostics.is_empty(), "EXP {name}");
        let counts = d.counts();
        assert_eq!(counts.trims, trims, "EXP {name} trims");
        assert_eq!(counts.jumps, jumps, "EXP {name} jumps");
        assert_eq!(counts.stitches, 271, "EXP {name} stitches");
        assert_eq!(counts.color_changes, 0, "EXP {name}");
        // No trim moves the needle.
        assert!(
            d.commands
                .iter()
                .filter(|c| matches!(c, Command::Trim { .. }))
                .all(|c| c.delta() == (0, 0)),
            "EXP {name}"
        );
    }
}

#[test]
fn needle_path_is_unaffected_by_trims() {
    let a = exp::decode(EXP_A).unwrap();
    let b = exp::decode(EXP_B).unwrap();
    // The two producers split the travel differently, but the needle
    // penetrates the same 271 points.
    let pa = penetrations(&a);
    assert_eq!(pa, penetrations(&b));
    assert_eq!(pa[0], (FIRST_BLOCK_X, 0));
    assert_eq!(pa.last(), Some(&(LAST_PENETRATION_X, 0)));
    // A travels home after its last trim. Were the payload `07 00` a
    // displacement, its 11 trims would leave it 77 units off.
    assert_eq!(a.positions().last(), Some(&(0, 0)));
    assert_eq!(b.positions().last(), Some(&(LAST_PENETRATION_X, 0)));

    // The PEC renderings sew the same points. B's PEC keeps the EXP
    // origin; A's starts at its first block and drops the opening
    // zero-length stitch.
    let pec_a = pec::decode(PEC_A).unwrap().design;
    let pec_b = pec::decode(PEC_B).unwrap().design;
    assert_eq!(penetrations(&pec_b), pa);
    assert_eq!(shifted(&penetrations(&pec_a), FIRST_BLOCK_X), pa[1..]);
}

#[test]
fn trims_fall_where_the_pec_renderings_trim() {
    let a = exp::decode(EXP_A).unwrap();
    let b = exp::decode(EXP_B).unwrap();
    let pec_a = pec::decode(PEC_A).unwrap().design;
    let pec_b = pec::decode(PEC_B).unwrap().design;

    // One trim at the end of each block.
    let expected: Vec<(i32, i32)> = (0..BLOCKS)
        .map(|k| (FIRST_BLOCK_X + k * PITCH, 0))
        .collect();
    assert_eq!(trim_sites(&b), expected);
    assert_eq!(trim_sites(&pec_b), expected);
    assert_eq!(shifted(&trim_sites(&pec_a), FIRST_BLOCK_X), expected);

    // A carries the same ten, plus one after the final stitch (before
    // its travel home) that no other rendering has.
    let sites_a = trim_sites(&a);
    assert_eq!(sites_a[..10], expected[..]);
    assert_eq!(sites_a[10..], [(LAST_PENETRATION_X, 0)]);

    // PEC folds the travel to the next block into the trim; EXP
    // writes it as the records after the trim. Both arrive at the
    // same place.
    for d in [&pec_a, &pec_b] {
        let moves: Vec<_> = d
            .commands
            .iter()
            .filter(|c| matches!(c, Command::Trim { .. }))
            .map(Command::delta)
            .collect();
        assert_eq!(moves[..9], [(PITCH, 0); 9]);
        assert_eq!(moves[9], (LAST_PENETRATION_X - expected[9].0, 0));
    }
}

#[test]
fn trim_fixtures_round_trip() {
    for (name, data) in [("A", EXP_A), ("B", EXP_B)] {
        let d = exp::decode(data).unwrap();
        // Both files are byte-exact fixed points of decode → encode.
        assert_eq!(exp::encode(&d).unwrap(), data, "EXP {name}");
        // Without trim records the cuts are gone and the path stays.
        let plain =
            exp::encode_with(&d, &exp::ExpEncodeOptions::new().with_trim_records(false)).unwrap();
        let plain = exp::decode_with(&plain, &DecodeOptions::strict()).unwrap();
        assert_eq!(plain.counts().trims, 0, "EXP {name}");
        assert_eq!(penetrations(&plain), penetrations(&d), "EXP {name}");
    }
}

#[test]
fn pec_trims_survive_conversion_to_exp() {
    for (name, data) in [("A", PEC_A), ("B", PEC_B)] {
        let pec_design = pec::decode(data).unwrap().design;
        let bytes = exp::encode(&pec_design).unwrap();
        let d = exp::decode_with(&bytes, &DecodeOptions::strict()).unwrap();
        assert_eq!(d.counts().trims, 10, "PEC {name}");
        assert_eq!(trim_sites(&d), trim_sites(&pec_design), "PEC {name}");
        assert_eq!(penetrations(&d), penetrations(&pec_design), "PEC {name}");
        assert_eq!(
            d.positions().last(),
            pec_design.positions().last(),
            "PEC {name}"
        );
        // And back: the EXP decode re-encodes to the same bytes.
        assert_eq!(exp::encode(&d).unwrap(), bytes, "PEC {name}");
    }
}
