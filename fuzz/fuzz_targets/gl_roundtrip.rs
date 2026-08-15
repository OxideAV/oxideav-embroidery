//! Structure-aware GL property: both encoders round-trip arbitrary
//! payloads bit-exactly at every window level, streams end exactly on
//! the end-of-stream code, and a level-w stream decodes under any
//! decoder window >= w.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxideav_embroidery::gl;

fuzz_target!(|data: &[u8]| {
    let Some((&sel, payload)) = data.split_first() else {
        return;
    };
    let window = 1024usize << (sel as usize % 5);
    let cap = payload.len().max(16);

    let lz = gl::compress_lz(payload, window).unwrap();
    let (got, used) = gl::decompress_with_window(&lz, window, cap).unwrap();
    assert_eq!(got, payload);
    assert_eq!(used, lz.len());
    // The default window (16384) is the largest level and must decode
    // any level's stream.
    let (dflt, _) = gl::decompress(&lz, cap).unwrap();
    assert_eq!(dflt, payload);

    let lit = gl::compress(payload);
    let (got, used) = gl::decompress(&lit, cap).unwrap();
    assert_eq!(got, payload);
    assert_eq!(used, lit.len());
});
