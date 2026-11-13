//! EXP stitch-list decode over arbitrary bytes, with the properties
//! the decoder promises:
//!
//! - lenient and strict decode agree: strict succeeds exactly when
//!   the lenient decode raised no warning, with the same commands;
//! - an unevidenced even control code never moves the needle: the
//!   needle path equals the one integrated from the raw records with
//!   those records dropped;
//! - re-encoding the decoded design yields a strict-clean stream
//!   ending at the same needle position.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxideav_embroidery::{Command, DecodeOptions, WarningAction, exp};

fuzz_target!(|data: &[u8]| {
    let Ok(design) = exp::decode(data) else {
        assert!(exp::decode_with(data, &DecodeOptions::strict()).is_err());
        return;
    };
    match exp::decode_with(data, &DecodeOptions::strict()) {
        Ok(strict) => {
            assert!(design.diagnostics.is_empty());
            assert_eq!(strict.commands, design.commands);
        }
        Err(_) => assert!(!design.diagnostics.is_empty()),
    }
    assert!(design.diagnostics.warnings().len() <= design.diagnostics.total());

    // Independent walk of the raw records.
    let (mut x, mut y, mut skipped) = (0i64, 0i64, 0usize);
    let mut i = 0usize;
    while i < data.len() {
        let (dx, dy, len) = if data[i] == 0x80 {
            let code = data[i + 1];
            if code.is_multiple_of(2) && code != 0x02 && code != 0x04 {
                skipped += 1;
                (0, 0, 4)
            } else {
                (data[i + 2] as i8, data[i + 3] as i8, 4)
            }
        } else {
            (data[i] as i8, data[i + 1] as i8, 2)
        };
        x += dx as i64;
        y += dy as i64;
        i += len;
    }
    let end = design.positions().last().copied().unwrap_or((0, 0));
    assert_eq!((end.0 as i64, end.1 as i64), (x, y));
    assert!(skipped <= design.diagnostics.total());
    if design.diagnostics.total() <= oxideav_embroidery::Diagnostics::MAX_RECORDED {
        let listed = design
            .diagnostics
            .warnings()
            .iter()
            .filter(|w| w.action == WarningAction::Skipped)
            .count();
        assert_eq!(listed, skipped);
    }

    // The writer caps design extents; a stream beyond the cap is a
    // clean error, not a property failure.
    let Ok(bytes) = exp::encode(&design) else {
        return;
    };
    let again = exp::decode_with(&bytes, &DecodeOptions::strict()).unwrap();
    assert_eq!(again.positions().last().copied().unwrap_or((0, 0)), end);
    assert!(matches!(again.commands.last(), Some(Command::End)));
});
