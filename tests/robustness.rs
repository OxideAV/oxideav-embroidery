//! Decoder robustness: no decoder may panic on arbitrary or
//! corrupted input — every outcome must be a clean `Ok`/`Err`.
//!
//! Deterministic (seeded LCG) so failures reproduce: random buffers,
//! byte-flipped mutations of valid encodes, and truncations at every
//! interesting boundary.

use oxideav_embroidery::{dst, exp, hus, jef, pec, pes, phc, phx, vp3, Command, Design};

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
    let _ = phx::decode(data);
    let _ = exp::decode(data);
    let _ = exp::decode_inf(data);
    let _ = jef::decode(data);
    let _ = hus::parse(data);
    let _ = vp3::parse(data);
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
    Design {
        commands,
        ..Default::default()
    }
}

fn valid_encodes() -> Vec<Vec<u8>> {
    let d = valid_design();
    vec![
        dst::encode(&d, &dst::DstEncodeOptions::default()).unwrap(),
        pec::encode(&d, &pec::PecEncodeOptions::default()).unwrap(),
        pes::encode(&d, &pec::PecEncodeOptions::default()).unwrap(),
        exp::encode(&d).unwrap(),
        jef::encode(&d, &jef::JefEncodeOptions::default()).unwrap(),
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
        b"#PHX0200",
        b"LA:x    ",
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
    let d = Design {
        commands: vec![
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
        ],
        ..Default::default()
    };
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
}
