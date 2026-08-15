//! Structure-aware HUS container fuzzing: wrap the input in a
//! syntactically valid header (magic, counts, ascending offsets) so
//! the GL layer and the stitch rebuilder are always reached instead
//! of dying at the magic check.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxideav_embroidery::hus;

fuzz_target!(|data: &[u8]| {
    let Some((&records_hint, body)) = data.split_first() else {
        return;
    };
    let third = body.len() / 3;
    let (a, rest) = body.split_at(third);
    let (x, y) = rest.split_at(third);

    // Colour table (1 entry) + the two filler bytes.
    let base = 0x28 + 2 + 2;
    let mut f = Vec::with_capacity(base + body.len());
    f.extend_from_slice(&hus::MAGIC.to_le_bytes());
    f.extend_from_slice(&u32::from(records_hint).to_le_bytes());
    f.extend_from_slice(&1u32.to_le_bytes());
    f.extend_from_slice(&[0u8; 8]); // extents
    f.extend_from_slice(&(base as u32).to_le_bytes());
    f.extend_from_slice(&((base + a.len()) as u32).to_le_bytes());
    f.extend_from_slice(&((base + a.len() + x.len()) as u32).to_le_bytes());
    f.extend_from_slice(b"FUZZLBL\0");
    f.extend_from_slice(&5u16.to_le_bytes());
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(a);
    f.extend_from_slice(x);
    f.extend_from_slice(y);
    let _ = hus::decode(&f);

    // The same body behind the VIP magic exercises the inserted-record
    // parser against arbitrary gap bytes.
    let _ = hus::parse_vip(&{
        let mut v = f.clone();
        v[..4].copy_from_slice(&hus::VIP_MAGIC.to_le_bytes());
        v
    });
});
