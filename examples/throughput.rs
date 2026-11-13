//! Decode / encode throughput per format, in million records per
//! second (median of several runs, single thread).
//!
//! ```text
//! cargo run --release --example throughput            # synthetic design
//! cargo run --release --example throughput -- <dir>   # plus every design file under <dir>
//! ```

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use oxideav_embroidery::dst::DstEncodeOptions;
use oxideav_embroidery::hus::HusEncodeOptions;
use oxideav_embroidery::jef::JefEncodeOptions;
use oxideav_embroidery::pec::PecEncodeOptions;
use oxideav_embroidery::{Command, Design, dst, exp, hus, jef, pec, pes};

const RUNS: usize = 9;

/// A deterministic design shaped like real embroidery: runs of short
/// stitches, a jump every few hundred records, a trim before most
/// jumps, and a handful of colour blocks.
fn synthetic(records: usize, colors: usize) -> Design {
    let mut state = 0x1234_5678u32;
    let mut next = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        state >> 8
    };
    let per_color = records / colors;
    let mut commands = Vec::with_capacity(records + 1);
    let (mut x, mut y) = (0i32, 0i32);
    for i in 0..records {
        let r = next();
        // Keep the needle inside a ±100 mm field.
        let pull = |v: i32, d: i32| {
            if v.abs() > 900 {
                -v.signum() * d.abs()
            } else {
                d
            }
        };
        let (dx, dy) = (
            pull(x, (r % 61) as i32 - 30),
            pull(y, ((r >> 8) % 61) as i32 - 30),
        );
        let c = if i > 0 && i % per_color == 0 && i / per_color < colors {
            Command::ColorChange {
                dx: 0,
                dy: 0,
                index: None,
            }
        } else if i % 400 == 399 {
            Command::Trim { dx: 0, dy: 0 }
        } else if i % 400 == 0 {
            Command::Jump { dx, dy }
        } else {
            Command::Stitch { dx, dy }
        };
        let (mx, my) = c.delta();
        x += mx;
        y += my;
        commands.push(c);
    }
    commands.push(Command::End);
    Design::default().with_commands(commands)
}

/// Median wall time of `f` in seconds.
fn median(mut f: impl FnMut()) -> f64 {
    let mut times: Vec<f64> = (0..RUNS)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64()
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[RUNS / 2]
}

fn report(name: &str, what: &str, records: usize, secs: f64) {
    println!(
        "{name:<5} {what:<7} {:>8.1} M records/s  ({records} records, {:.2} ms)",
        records as f64 / secs / 1e6,
        secs * 1e3
    );
}

fn bench_format(
    name: &str,
    design: &Design,
    encode: impl Fn(&Design) -> Vec<u8>,
    decode: impl Fn(&[u8]) -> Design,
) {
    let bytes = encode(design);
    let records = decode(&bytes).commands.len();
    report(
        name,
        "decode",
        records,
        median(|| drop(black_box(decode(black_box(&bytes))))),
    );
    report(
        name,
        "encode",
        design.commands.len(),
        median(|| drop(black_box(encode(black_box(design))))),
    );
}

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn main() {
    let design = synthetic(1_000_000, 8);
    println!(
        "synthetic design: {} records, 8 colours",
        design.commands.len()
    );
    bench_format(
        "EXP",
        &design,
        |d| exp::encode(d).unwrap(),
        |b| exp::decode(b).unwrap(),
    );
    bench_format(
        "DST",
        &design,
        |d| dst::encode(d, &DstEncodeOptions::default()).unwrap(),
        |b| dst::decode(b).unwrap().design,
    );
    bench_format(
        "JEF",
        &design,
        |d| jef::encode(d, &JefEncodeOptions::default()).unwrap(),
        |b| jef::decode(b).unwrap().design,
    );
    bench_format(
        "PEC",
        &design,
        |d| pec::encode(d, &PecEncodeOptions::default()).unwrap(),
        |b| pec::decode(b).unwrap().design,
    );
    bench_format(
        "PES",
        &design,
        |d| pes::encode(d, &PecEncodeOptions::default()).unwrap(),
        |b| pes::decode(b).unwrap().pec.design,
    );
    bench_format(
        "HUS",
        &design,
        |d| hus::encode(d, &HusEncodeOptions::default()).unwrap(),
        |b| hus::decode(b).unwrap().design,
    );
    bench_format(
        "VIP",
        &design,
        |d| hus::encode_vip(d, &HusEncodeOptions::default()).unwrap(),
        |b| hus::decode(b).unwrap().design,
    );

    // Real files: total records decoded per second, per extension.
    let Some(root) = std::env::args().nth(1) else {
        return;
    };
    let mut by_ext = std::collections::BTreeMap::<String, Vec<Vec<u8>>>::new();
    for path in files_under(Path::new(&root)) {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        // Only the extensions this crate decodes stitches for (the
        // tape-family siblings share the DST header, not its records).
        const DECODED: [&str; 12] = [
            "dst", "exp", "jef", "ptn", "jef+", "jpx", "pec", "pes", "phc", "phb", "hus", "vip",
        ];
        if !DECODED.contains(&ext.as_str()) {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let ok = if ext == "exp" {
            exp::decode(&data).is_ok()
        } else {
            oxideav_embroidery::decode(&data).is_ok()
        };
        if ok {
            by_ext.entry(ext).or_default().push(data);
        }
    }
    println!("files under {root}:");
    for (ext, files) in &by_ext {
        let decode_all = || -> usize {
            files
                .iter()
                .map(|data| {
                    if ext == "exp" {
                        exp::decode(data).unwrap().commands.len()
                    } else {
                        oxideav_embroidery::decode(data).unwrap().1.commands.len()
                    }
                })
                .sum()
        };
        let records = decode_all();
        let secs = median(|| {
            black_box(decode_all());
        });
        report(ext, "decode", records, secs);
    }
}
