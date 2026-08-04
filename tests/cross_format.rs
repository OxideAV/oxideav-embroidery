//! Cross-format agreement tests.
//!
//! The staged corpus validation (docs/embroidery/provenance/05, 06,
//! 07) established that one design encoded into many formats decodes
//! to the same stitch list. The purchased corpus itself cannot be
//! committed (copyrighted commercial designs), so these tests apply
//! the same methodology to a self-synthesized design: encode one
//! model into every writable format, decode each file back, and
//! require agreement on the sewn path, the colour structure and the
//! bounding box.

use oxideav_embroidery::{dst, exp, jef, pec, pes, Command, Design, Format, Thread};

/// A deterministic multi-colour design exercising stitches, jumps
/// (including ones large enough to split in every format), trims and
/// colour changes. Stitch deltas stay within every format's
/// single-record range so the sewn path is format-independent.
fn reference_design() -> Design {
    let mut commands = Vec::new();
    let mut state = 0x2F6E_2B1Eu32;
    let mut rng = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        state
    };
    commands.push(Command::Jump { dx: 400, dy: -250 });
    for block in 0..3 {
        for i in 0..120 {
            let dx = (rng() % 201) as i32 - 100;
            let dy = (rng() % 201) as i32 - 100;
            commands.push(Command::Stitch { dx, dy });
            if i % 40 == 39 {
                commands.push(Command::Trim { dx: 0, dy: 0 });
                commands.push(Command::Jump {
                    dx: (rng() % 301) as i32 - 150,
                    dy: (rng() % 301) as i32 - 150,
                });
            }
        }
        if block < 2 {
            commands.push(Command::ColorChange {
                dx: 0,
                dy: 0,
                index: None,
            });
        }
    }
    commands.push(Command::End);
    Design {
        commands,
        threads: vec![
            Thread {
                palette_index: Some(20),
                ..Default::default()
            },
            Thread {
                palette_index: Some(5),
                ..Default::default()
            },
            Thread {
                palette_index: Some(2),
                ..Default::default()
            },
        ],
        label: None,
    }
}

/// The absolute positions at which a stitch is sewn — the
/// format-independent content of a design.
fn sewn_path(d: &Design) -> Vec<(i32, i32)> {
    let (mut x, mut y) = (0i32, 0i32);
    let mut path = Vec::new();
    for c in &d.commands {
        let (dx, dy) = c.delta();
        x += dx;
        y += dy;
        if matches!(c, Command::Stitch { .. }) {
            path.push((x, y));
        }
    }
    path
}

#[test]
fn all_formats_agree_on_the_sewn_path() {
    let d = reference_design();
    let reference_path = sewn_path(&d);
    let reference_extents = d.extents();
    assert!(reference_path.len() >= 360);

    let dst_file = dst::decode(&dst::encode(&d, &dst::DstEncodeOptions::default()).unwrap())
        .unwrap()
        .design;
    let pec_file = pec::decode(&pec::encode(&d, &pec::PecEncodeOptions::default()).unwrap())
        .unwrap()
        .design;
    let pes_file = pes::decode(&pes::encode(&d, &pec::PecEncodeOptions::default()).unwrap())
        .unwrap()
        .pec
        .design;
    let exp_file = exp::decode(&exp::encode(&d).unwrap()).unwrap();
    let jef_file = jef::decode(&jef::encode(&d, &jef::JefEncodeOptions::default()).unwrap())
        .unwrap()
        .design;

    for (name, decoded) in [
        ("DST", &dst_file),
        ("PEC", &pec_file),
        ("PES", &pes_file),
        ("EXP", &exp_file),
        ("JEF", &jef_file),
    ] {
        assert_eq!(sewn_path(decoded), reference_path, "sewn path via {name}");
        assert_eq!(
            decoded.counts().color_changes,
            2,
            "colour changes via {name}"
        );
        let e = decoded.extents();
        assert_eq!(
            (e.min_x, e.max_x, e.min_y, e.max_y),
            (
                reference_extents.min_x,
                reference_extents.max_x,
                reference_extents.min_y,
                reference_extents.max_y
            ),
            "extents via {name}"
        );
    }

    // PEC carries the trims through explicitly.
    assert_eq!(pec_file.counts().trims, d.counts().trims);
    // Every format preserves the stitch count (deltas fit one record).
    for decoded in [&dst_file, &pec_file, &pes_file, &exp_file, &jef_file] {
        assert_eq!(decoded.counts().stitches, d.counts().stitches);
    }
}

#[test]
fn dst_header_totals_match_decoded_content() {
    let d = reference_design();
    let bytes = dst::encode(&d, &dst::DstEncodeOptions::default()).unwrap();
    let f = dst::decode(&bytes).unwrap();
    let c = f.design.counts();
    assert_eq!(
        f.header.stitch_records.unwrap() as usize,
        c.stitches + c.jumps + c.color_changes + 1
    );
    assert_eq!(f.header.color_changes.unwrap() as usize, c.color_changes);
    let e = f.design.extents();
    assert_eq!(f.header.pos_x.unwrap(), e.max_x.max(0));
    assert_eq!(f.header.neg_x.unwrap(), (-e.min_x).max(0));
}

#[test]
fn pec_width_height_are_span_plus_one() {
    let d = reference_design();
    let p = pec::decode(&pec::encode(&d, &pec::PecEncodeOptions::default()).unwrap()).unwrap();
    let e = d.extents();
    assert_eq!(p.width as i32, e.width() + 1);
    assert_eq!(p.height as i32, e.height() + 1);
}

#[test]
fn probe_dispatch_identifies_every_signature_format() {
    let d = reference_design();
    let dst_bytes = dst::encode(&d, &dst::DstEncodeOptions::default()).unwrap();
    let pec_bytes = pec::encode(&d, &pec::PecEncodeOptions::default()).unwrap();
    let pes_bytes = pes::encode(&d, &pec::PecEncodeOptions::default()).unwrap();
    let jef_bytes = jef::encode(&d, &jef::JefEncodeOptions::default()).unwrap();
    assert_eq!(oxideav_embroidery::probe(&dst_bytes), Some(Format::Dst));
    assert_eq!(oxideav_embroidery::probe(&pec_bytes), Some(Format::Pec));
    assert_eq!(oxideav_embroidery::probe(&pes_bytes), Some(Format::Pes));
    assert_eq!(oxideav_embroidery::probe(&jef_bytes), Some(Format::Jef));
    assert_eq!(oxideav_embroidery::probe(b"not an embroidery file"), None);

    // Signature-bearing formats whose full decode is elsewhere:
    // magic-only buffers must still probe to the right family.
    let mut phb_head = b"#PHB0003".to_vec();
    phb_head.resize(64, 0);
    assert_eq!(oxideav_embroidery::probe(&phb_head), Some(Format::Phb));
    let mut vip_head = 0x0190_FC5Du32.to_le_bytes().to_vec();
    vip_head.resize(64, 0);
    assert_eq!(oxideav_embroidery::probe(&vip_head), Some(Format::Vip));

    let (fmt, decoded) = oxideav_embroidery::decode(&pes_bytes).unwrap();
    assert_eq!(fmt, Format::Pes);
    assert_eq!(sewn_path(&decoded), sewn_path(&d));
}

#[test]
fn palette_flows_from_threads_into_brother_formats() {
    let d = reference_design();
    let p = pec::decode(&pec::encode(&d, &pec::PecEncodeOptions::default()).unwrap()).unwrap();
    // Thread palette indices (20, 5, 2) come from the model.
    assert_eq!(p.palette, vec![20, 5, 2]);
    assert_eq!(p.design.threads[0].name.as_deref(), Some("Black"));
    assert_eq!(p.design.threads[1].name.as_deref(), Some("Red"));
    assert_eq!(p.design.threads[2].name.as_deref(), Some("Blue"));
}
