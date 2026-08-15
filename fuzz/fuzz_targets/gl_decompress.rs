//! Hostile raw GL bitstreams straight into the decompressor: no
//! panic, allocation bounded by the caller's cap, consumed bytes
//! within the input — and anything that decodes must survive a
//! literal-only re-compression round-trip.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxideav_embroidery::gl;

fuzz_target!(|data: &[u8]| {
    if let Ok((out, used)) = gl::decompress(data, 1 << 16) {
        assert!(out.len() <= 1 << 16);
        assert!(used <= data.len());
        let (again, _) = gl::decompress(&gl::compress(&out), out.len().max(16)).unwrap();
        assert_eq!(again, out);
    }
    // Every documented window level must behave, not just the default.
    for window in [1024usize, 2048, 4096, 8192] {
        let _ = gl::decompress_with_window(data, window, 1 << 14);
    }
});
