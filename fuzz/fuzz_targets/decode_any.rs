//! Every public decode entry point of the crate over arbitrary
//! bytes: every outcome must be a clean Ok/Err — no panic, no
//! unbounded allocation.

#![no_main]

use libfuzzer_sys::fuzz_target;
use oxideav_embroidery::{art, col, dst, edr, exp, gl, hus, jef, pec, pes, phc, phx, vp3, xxx};

fuzz_target!(|data: &[u8]| {
    let _ = oxideav_embroidery::probe(data);
    let _ = oxideav_embroidery::decode(data);
    let _ = dst::decode(data);
    let _ = dst::decode_header(data);
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
    let _ = xxx::parse(data);
    let _ = col::decode(data);
    let _ = edr::decode(data);
    let _ = art::decode_design(data);
    let _ = gl::decompress(data, 1 << 16);
    // The framework demuxer applies its own probe + validated-walk
    // fallback on top of the raw decoders.
    let _ = oxideav_embroidery::framework::EmbroideryDemuxer::from_bytes(data.to_vec());
});
