//! Decoder robustness: no decoder may panic on arbitrary or
//! corrupted input — every outcome must be a clean `Ok`/`Err`.
//!
//! Deterministic (seeded LCG) so failures reproduce: random buffers,
//! byte-flipped mutations of valid encodes, and truncations at every
//! interesting boundary.

use oxideav_embroidery::{
    Command, Design, art, col, dst, edr, exp, gl, hus, jef, pec, pes, phc, phx, vp3, xxx,
};

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
}

fn try_all_decoders(data: &[u8]) {
    let _ = oxideav_embroidery::probe(data);
    let _ = oxideav_embroidery::decode(data);
    let _ = dst::decode(data);
    let _ = pec::decode(data);
    let _ = pec::decode_block(data);
    let _ = pes::decode(data);
    let _ = phc::decode(data);
    let _ = phc::decode_phb(data);
    let _ = phx::decode(data);
    let _ = exp::decode(data);
    let _ = exp::decode_inf(data);
    let _ = jef::decode(data);
    let _ = hus::parse(data);
    let _ = hus::parse_vip(data);
    let _ = hus::decode(data);
    let _ = vp3::parse(data);
    // XXX re-serialisation is lossless for anything that parses.
    if let Ok(f) = xxx::parse(data) {
        assert_eq!(xxx::write(&f).unwrap(), data);
    }
    let _ = col::decode(data);
    let _ = edr::decode(data);
    let _ = art::decode_design(data);
    // The GL decompressor sees hostile bitstreams directly; the
    // output cap bounds allocation.
    let _ = gl::decompress(data, 1 << 16);
}

fn valid_design() -> Design {
    let mut commands = vec![Command::Jump { dx: 80, dy: -40 }];
    for i in 0..60 {
        commands.push(Command::Stitch {
            dx: (i % 13) - 6,
            dy: (i % 9) - 4,
        });
    }
    commands.push(Command::Trim { dx: 0, dy: 0 });
    commands.push(Command::ColorChange {
        dx: 0,
        dy: 0,
        index: None,
    });
    commands.push(Command::Stitch { dx: 5, dy: 5 });
    commands.push(Command::End);
    Design::default().with_commands(commands)
}

fn valid_encodes() -> Vec<Vec<u8>> {
    let d = valid_design();
    let threads = vec![
        oxideav_embroidery::Thread::default()
            .with_palette_index(Some(1))
            .with_rgb(Some([0, 254, 0])),
        oxideav_embroidery::Thread::default()
            .with_palette_index(Some(2))
            .with_rgb(Some([0, 28, 223])),
    ];
    // A match-bearing GL stream (the documented match/offset layer no
    // real producer emits) as a mutation/truncation seed for the raw
    // decompressor in `try_all_decoders`.
    let repetitive: Vec<u8> = (0..4000u32).map(|i| (i * 7 % 97) as u8).collect();
    vec![
        dst::encode(&d, &dst::DstEncodeOptions::default()).unwrap(),
        pec::encode(&d, &pec::PecEncodeOptions::default()).unwrap(),
        pes::encode(&d, &pec::PecEncodeOptions::default()).unwrap(),
        exp::encode(&d).unwrap(),
        jef::encode(&d, &jef::JefEncodeOptions::default()).unwrap(),
        exp::encode_inf(&threads).unwrap(),
        col::encode(&threads).unwrap(),
        edr::encode(&threads),
        hus::encode(&d, &hus::HusEncodeOptions::default()).unwrap(),
        hus::encode_vip(&d, &hus::HusEncodeOptions::default()).unwrap(),
        gl::compress_lz(&repetitive, 1024).unwrap(),
    ]
}

#[test]
fn random_buffers_do_not_panic() {
    let mut rng = Lcg(0xDEAD_BEEF);
    for len in [0usize, 1, 3, 8, 12, 100, 513, 600, 2048] {
        for _ in 0..40 {
            let buf: Vec<u8> = (0..len).map(|_| (rng.next() >> 13) as u8).collect();
            try_all_decoders(&buf);
        }
    }
}

#[test]
fn magic_prefixed_random_tails_do_not_panic() {
    let mut rng = Lcg(0x0BAD_F00D);
    for magic in [
        b"#PES0001".as_slice(),
        b"#PEC0001",
        b"#PHC0001",
        b"#PHB0003",
        b"#PHX0200",
        b"LA:x    ",
        b"\x5B\xAF\xC8\x00",
        b"\x5D\xFC\x90\x01",
        b"%vsm%",
    ] {
        for len in [0usize, 4, 40, 600, 1500] {
            for _ in 0..25 {
                let mut buf = magic.to_vec();
                buf.extend((0..len).map(|_| (rng.next() >> 13) as u8));
                try_all_decoders(&buf);
            }
        }
    }
}

#[test]
fn mutated_valid_files_do_not_panic() {
    let mut rng = Lcg(0x5EED_5EED);
    for original in valid_encodes() {
        for _ in 0..300 {
            let mut buf = original.clone();
            let flips = 1 + (rng.next() % 4) as usize;
            for _ in 0..flips {
                let pos = (rng.next() as usize) % buf.len();
                buf[pos] ^= (rng.next() >> 9) as u8;
            }
            try_all_decoders(&buf);
        }
    }
}

#[test]
fn truncations_do_not_panic() {
    for original in valid_encodes() {
        // Every prefix for small files, sampled boundaries for larger.
        let step = (original.len() / 200).max(1);
        for cut in (0..original.len()).step_by(step) {
            try_all_decoders(&original[..cut]);
        }
    }
}

#[test]
fn extents_saturate_instead_of_overflowing() {
    let d = Design::default().with_commands(vec![
        Command::Jump {
            dx: i32::MAX,
            dy: i32::MAX,
        },
        Command::Jump {
            dx: i32::MAX,
            dy: i32::MAX,
        },
        Command::Jump {
            dx: i32::MIN,
            dy: i32::MIN,
        },
        Command::End,
    ]);
    let e = d.extents();
    assert_eq!(e.max_x, i32::MAX);
    assert_eq!(e.min_x, 0);
    let p = d.positions();
    assert_eq!(p[1], (i32::MAX, i32::MAX));
    // Encoders reject such designs cleanly instead of looping.
    assert!(dst::encode(&d, &dst::DstEncodeOptions::default()).is_err());
    assert!(pec::encode(&d, &pec::PecEncodeOptions::default()).is_err());
    assert!(exp::encode(&d).is_err());
    assert!(jef::encode(&d, &jef::JefEncodeOptions::default()).is_err());
    assert!(hus::encode(&d, &hus::HusEncodeOptions::default()).is_err());
}

/// EXP over random record streams dense in control escapes: strict
/// and lenient decodes agree, and a control code without file
/// evidence never moves the needle — the decoded end position equals
/// an independent walk of the raw records with those records dropped.
#[test]
fn exp_unevidenced_controls_never_move_the_needle() {
    use oxideav_embroidery::{DecodeOptions, WarningAction};
    let mut rng = Lcg(0x5EED_0E0E);
    for _ in 0..2000 {
        let records = (rng.next() % 200) as usize;
        let mut data = Vec::new();
        for _ in 0..records {
            let r = rng.next();
            if r.is_multiple_of(4) {
                data.extend_from_slice(&[0x80, (r >> 8) as u8, (r >> 16) as u8, (r >> 24) as u8]);
            } else {
                let dx = (r >> 8) as u8;
                data.extend_from_slice(&[if dx == 0x80 { 0x7F } else { dx }, (r >> 16) as u8]);
            }
        }
        let design = exp::decode(&data).unwrap();
        let (mut x, mut y, mut skipped) = (0i32, 0i32, 0usize);
        let mut i = 0usize;
        while i < data.len() {
            if data[i] != 0x80 {
                x += data[i] as i8 as i32;
                y += data[i + 1] as i8 as i32;
                i += 2;
                continue;
            }
            let code = data[i + 1];
            if code % 2 == 1 || code == 0x02 || code == 0x04 {
                x += data[i + 2] as i8 as i32;
                y += data[i + 3] as i8 as i32;
            } else {
                skipped += 1;
            }
            i += 4;
        }
        assert_eq!(design.positions().last().copied(), Some((x, y)));
        let listed_skips = design
            .diagnostics
            .warnings()
            .iter()
            .filter(|w| w.action == WarningAction::Skipped)
            .count();
        assert!(listed_skips <= skipped && skipped <= design.diagnostics.total());
        let strict = exp::decode_with(&data, &DecodeOptions::strict());
        assert_eq!(strict.is_ok(), design.diagnostics.is_empty());
        // What the writer produces from the decode is strict-clean
        // and ends at the same needle position.
        let again =
            exp::decode_with(&exp::encode(&design).unwrap(), &DecodeOptions::strict()).unwrap();
        assert_eq!(again.positions().last().copied(), Some((x, y)));
    }
}
