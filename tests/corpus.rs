//! Corpus-gated validation against real purchased design files.
//!
//! The workspace stages a commercial validation corpus (three
//! designs, ~62 formats, private-repo-only tier — see
//! `docs/embroidery/reference/corpus/` and `corpus-map.md` in the
//! workspace's docs repository). Those files are copyrighted and are
//! **never** committed to this repository; these tests locate the
//! corpus at runtime and **skip silently when it is absent** (all
//! assertions become no-ops), so CI stays green without the corpus
//! while local runs against the staged files re-validate every
//! decoder against real machine output.
//!
//! Point `OXIDEAV_EMBROIDERY_CORPUS` at the corpus root, or run
//! inside the umbrella workspace where the default staging path
//! exists.

use std::path::{Path, PathBuf};

use oxideav_embroidery::{dst, exp, gl, hus, Command, Format};

fn corpus_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("OXIDEAV_EMBROIDERY_CORPUS") {
        let p = PathBuf::from(p);
        return p.is_dir().then_some(p);
    }
    // Umbrella-workspace staging path, relative to this crate.
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/embroidery/reference/corpus");
    p.is_dir().then_some(p)
}

/// Every corpus file with one of the given extensions.
fn corpus_files(exts: &[&str]) -> Vec<PathBuf> {
    let Some(root) = corpus_root() else {
        eprintln!("corpus not present; skipping");
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// HUS/VIP: every file decodes to exactly its declared record count,
/// each GL stream is consumed exactly, the integrated deltas
/// reproduce the header extents, and the colour-change count is
/// `colours − 1`.
#[test]
fn hus_vip_decode_exactly() {
    let files = corpus_files(&["hus", "vip"]);
    for path in &files {
        let data = std::fs::read(path).unwrap();
        let file = if hus::probe(&data) {
            hus::parse(&data).unwrap()
        } else {
            hus::parse_vip(&data).unwrap()
        };
        let n = file.stitch_count as usize;
        // Exact consumption of all three compressed regions. The
        // final (Y) region runs to end of file and some writers pad
        // it with a single trailing zero byte, so it alone gets one
        // byte of slack.
        for (raw, what, slack) in [
            (&file.attributes, "attr", 0usize),
            (&file.x_deltas, "x", 0),
            (&file.y_deltas, "y", 1),
        ] {
            let (out, used) = gl::decompress(raw, n).unwrap();
            assert_eq!(out.len(), n, "{} stream length in {}", what, path.display());
            assert!(
                raw.len() - used <= slack,
                "{} stream not exactly consumed in {} ({} of {} bytes)",
                what,
                path.display(),
                used,
                raw.len()
            );
        }
        let design = hus::decode_design(&file).unwrap();
        assert_eq!(design.commands.len(), n, "{}", path.display());
        assert!(matches!(design.commands.last(), Some(Command::End)));
        // Integrated extents reproduce the header extents exactly.
        let e = design.extents();
        assert_eq!(
            (e.max_x, e.max_y, e.min_x, e.min_y),
            (
                file.extents.0 as i32,
                file.extents.1 as i32,
                file.extents.2 as i32,
                file.extents.3 as i32
            ),
            "extents in {}",
            path.display()
        );
        assert_eq!(
            design.counts().color_changes + 1,
            file.palette.len(),
            "colour blocks in {}",
            path.display()
        );
    }
    if !files.is_empty() {
        assert!(files.len() >= 11, "expected the full 11-file HUS/VIP set");
    }
}

/// The staged docs' strongest claim: a HUS stitch stream is identical
/// record-for-record to the sibling Melco EXP of the same design,
/// after dropping the trim and end records EXP does not represent.
#[test]
fn hus_matches_exp_sibling_record_for_record() {
    let mut compared = 0usize;
    for path in corpus_files(&["hus"]) {
        let sibling = path.with_extension("exp");
        if !sibling.exists() {
            continue;
        }
        let h = hus::decode(&std::fs::read(&path).unwrap()).unwrap();
        let e = exp::decode(&std::fs::read(&sibling).unwrap()).unwrap();
        let hus_side: Vec<Command> = h
            .design
            .commands
            .iter()
            .filter(|c| !matches!(c, Command::Trim { .. } | Command::End))
            .copied()
            .collect();
        let exp_side: Vec<Command> = e
            .commands
            .iter()
            .filter(|c| !matches!(c, Command::End))
            .map(|c| match *c {
                // EXP colour-change records carry no index byte.
                Command::ColorChange { dx, dy, .. } => Command::ColorChange {
                    dx,
                    dy,
                    index: None,
                },
                other => other,
            })
            .collect();
        assert_eq!(
            hus_side.len(),
            exp_side.len(),
            "record counts differ for {}",
            path.display()
        );
        for (i, (a, b)) in hus_side.iter().zip(exp_side.iter()).enumerate() {
            assert_eq!(a, b, "record {i} differs for {}", path.display());
        }
        compared += hus_side.len();
    }
    if corpus_root().is_some() {
        assert!(compared > 100_000, "expected six-figure record comparison");
    }
}

/// Every signature-bearing corpus file probes to the right format and
/// decodes to a non-trivial design (VP3 is header-only and excluded).
#[test]
fn probe_and_decode_sweep() {
    let expected = [
        ("dst", Format::Dst),
        ("pes", Format::Pes),
        ("pec", Format::Pec),
        ("phc", Format::Phc),
        ("phb", Format::Phb),
        ("jef", Format::Jef),
        ("hus", Format::Hus),
        ("vip", Format::Vip),
    ];
    for (ext, want) in expected {
        for path in corpus_files(&[ext]) {
            let data = std::fs::read(&path).unwrap();
            assert_eq!(
                oxideav_embroidery::probe(&data),
                Some(want),
                "probe of {}",
                path.display()
            );
            let (format, design) = oxideav_embroidery::decode(&data).unwrap();
            assert_eq!(format, want);
            assert!(
                design.counts().stitches > 1000,
                "suspiciously small decode of {}",
                path.display()
            );
        }
    }
    // Headerless EXP decodes through its module.
    for path in corpus_files(&["exp"]) {
        let design = exp::decode(&std::fs::read(&path).unwrap()).unwrap();
        assert!(design.counts().stitches > 1000, "{}", path.display());
    }
    // The DST tape-family siblings share the 512-byte header.
    for path in corpus_files(&["dsb", "dsz"]) {
        let data = std::fs::read(&path).unwrap();
        let h = dst::decode_header(&data).unwrap();
        assert!(h.stitch_records.unwrap_or(0) > 0, "{}", path.display());
    }
}

/// One design, many formats: the decoded designs of every sibling
/// agree on the extent-magnitude set (the corpus contains rotated
/// exports, so magnitudes are compared as a set, per the staged
/// docs' orientation caveat) and on the colour-change count.
#[test]
fn cross_format_extent_agreement() {
    let Some(root) = corpus_root() else {
        eprintln!("corpus not present; skipping");
        return;
    };
    for design_dir in ["asterix-and-obelix", "flower-vine-pink-yellow"] {
        let dir = root.join(design_dir);
        if !dir.is_dir() {
            continue;
        }
        let mut seen: Vec<(String, [i32; 2], usize)> = Vec::new();
        for path in std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()) {
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !["dst", "pes", "pec", "phc", "phb", "jef", "hus", "vip"]
                .iter()
                .any(|x| ext.eq_ignore_ascii_case(x))
            {
                continue;
            }
            let Ok((_, design)) = oxideav_embroidery::decode(&std::fs::read(&path).unwrap()) else {
                continue;
            };
            let e = design.extents();
            let mut spans = [e.width(), e.height()];
            spans.sort();
            seen.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                spans,
                design.counts().color_changes,
            ));
        }
        // Files of the same design at the same size agree. These two
        // directories are single-size (asterix) or size-suffixed
        // (flower vine), so group by the stem before comparing.
        let mut by_stem: std::collections::BTreeMap<String, Vec<([i32; 2], usize)>> =
            Default::default();
        for (name, spans, ccs) in seen {
            let stem = name.rsplit_once('.').map(|(s, _)| s.to_owned()).unwrap();
            by_stem.entry(stem).or_default().push((spans, ccs));
        }
        for (stem, entries) in by_stem {
            assert!(!entries.is_empty());
            let (spans0, ccs0) = entries[0];
            for (spans, ccs) in &entries[1..] {
                assert_eq!(*spans, spans0, "extent span set for {stem}");
                assert_eq!(*ccs, ccs0, "colour changes for {stem}");
            }
        }
    }
}
