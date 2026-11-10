//! Fixture sweep over the workspace's redistributable 2026-09 fixture
//! round (`docs/embroidery/fixtures-2026-09/`: HUS / VIP with GL match
//! codes, PES / PEC section-1 bytes, XXX palette blocks).
//!
//! Every fixture directory carries a `notes.md` whose tables state, per
//! file, the facts measured on it (magic, stitch count, colour count,
//! per-stream match / literal mix, longest match and distance, bytes
//! after the end-of-stream code; PEC offset and section-1 bytes; XXX
//! palette escape, slot contents and trailer length). This test reads
//! those tables at runtime and requires every staged file to decode to
//! exactly what its `notes.md` states, then round-trips every file
//! through the crate's writers.
//!
//! The fixtures live in the workspace's docs repository, not in this
//! crate: the sweep **skips silently** when the directory is absent
//! (point `OXIDEAV_EMBROIDERY_FIXTURES` at it, or run inside the
//! umbrella workspace where the default path exists).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use oxideav_embroidery::pec::{self, PecEncodeOptions, PecLayout};
use oxideav_embroidery::xxx::{self, PaletteEscape};
use oxideav_embroidery::{Command, Design, gl, hus, pes};

fn fixtures_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("OXIDEAV_EMBROIDERY_FIXTURES") {
        let p = PathBuf::from(p);
        return p.is_dir().then_some(p);
    }
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/embroidery/fixtures-2026-09");
    p.is_dir().then_some(p)
}

/// The fixture directories of one format (`hus`, `pes`, `xxx`).
fn fixture_dirs(format: &str) -> Vec<PathBuf> {
    let Some(root) = fixtures_root() else {
        eprintln!("fixtures-2026-09 not present; skipping");
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join(format))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("notes.md").is_file())
        .collect();
    dirs.sort();
    dirs
}

/// One markdown table: header cells plus rows of cells, keyed by the
/// first column with backticks stripped.
struct Table {
    header: Vec<String>,
    rows: BTreeMap<String, Vec<String>>,
}

impl Table {
    fn col(&self, name: &str) -> usize {
        self.header
            .iter()
            .position(|h| h.starts_with(name))
            .unwrap_or_else(|| panic!("no column {name:?} in {:?}", self.header))
    }
}

fn cells(line: &str) -> Vec<String> {
    let t = line.trim().trim_start_matches('|').trim_end_matches('|');
    t.split('|').map(|c| c.trim().to_string()).collect()
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('`').to_string()
}

/// Every table in `notes.md` whose header begins `| File | <second> |`.
fn notes_table(dir: &Path, second: &str) -> Table {
    let text = std::fs::read_to_string(dir.join("notes.md")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i].trim();
        if l.starts_with('|') {
            let h = cells(l);
            if h.len() >= 2 && h[0] == "File" && h[1] == second {
                let mut rows = BTreeMap::new();
                let mut j = i + 2; // skip the separator row
                while j < lines.len() && lines[j].trim().starts_with('|') {
                    let c = cells(lines[j]);
                    rows.insert(unquote(&c[0]), c);
                    j += 1;
                }
                return Table { header: h, rows };
            }
        }
        i += 1;
    }
    panic!("no `File | {second}` table in {}", dir.display());
}

/// Every data file in a fixture directory (notes and licence texts
/// excluded), checked against the notes' size table.
fn data_files(dir: &Path) -> Vec<PathBuf> {
    let sizes = notes_table(dir, "Size");
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_str().unwrap();
            n != "notes.md" && !n.starts_with("LICENSE")
        })
        .collect();
    out.sort();
    for p in &out {
        let name = p.file_name().unwrap().to_str().unwrap();
        let row = sizes
            .rows
            .get(name)
            .unwrap_or_else(|| panic!("{name} missing from the notes' file table"));
        let want: u64 = row[1].parse().unwrap();
        assert_eq!(std::fs::metadata(p).unwrap().len(), want, "size of {name}");
    }
    assert_eq!(
        out.len(),
        sizes.rows.len(),
        "file table of {}",
        dir.display()
    );
    out
}

fn name_of(p: &Path) -> String {
    p.file_name().unwrap().to_str().unwrap().to_string()
}

fn hex_bytes(s: &str) -> Vec<u8> {
    unquote(s)
        .split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

fn rgb(s: &str) -> [u8; 3] {
    let s = unquote(s);
    let v = u32::from_str_radix(&s, 16).unwrap();
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn triple(s: &str) -> [usize; 3] {
    let v: Vec<usize> = s.split('/').map(|x| x.trim().parse().unwrap()).collect();
    [v[0], v[1], v[2]]
}

fn num(s: &str) -> usize {
    s.replace(',', "").parse().unwrap()
}

// ───────────────────────────── HUS / VIP ─────────────────────────────

/// Every HUS / VIP fixture: magic, stitch count, colour count, the
/// per-stream match / literal mix, longest match, longest distance,
/// distance-0 count, exact stream length, integrated extents and the
/// number of unused bytes after the end-of-stream code — all exactly as
/// the notes state. Then read → write → read with identical stitches,
/// for both the producer-shaped and the match-bearing encoder.
#[test]
fn hus_vip_fixtures_decode_to_notes() {
    let mut checked = 0;
    let mut no_end = 0;
    for dir in fixture_dirs("hus") {
        let t = notes_table(&dir, "Magic");
        for path in data_files(&dir) {
            let name = name_of(&path);
            let row = &t.rows[&name];
            let data = std::fs::read(&path).unwrap();
            let decoded = hus::decode(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
            let f = &decoded.file;
            let is_vip = name.to_ascii_lowercase().ends_with(".vip");
            assert_eq!(is_vip, hus::probe_vip(&data), "{name}");
            assert_eq!(!is_vip, hus::probe(&data), "{name}");
            assert_eq!(
                format!("{:08X}", f.magic),
                unquote(&row[t.col("Magic")]),
                "{name} magic"
            );
            let n = num(&row[t.col("Stitches")]);
            assert_eq!(f.stitch_count as usize, n, "{name} stitches");
            assert_eq!(
                f.palette.len(),
                num(&row[t.col("Colours")]),
                "{name} colours"
            );

            let matches = triple(&row[t.col("Matches")]);
            let literals = triple(&row[t.col("Literals")]);
            let after = triple(&row[t.col("Bytes after EOS")]);
            let (mut longest, mut far, mut zero) = (0, 0, 0);
            for (k, raw) in [&f.attributes, &f.x_deltas, &f.y_deltas]
                .into_iter()
                .enumerate()
            {
                let (out, st) = gl::decompress_stats(raw, n).unwrap();
                assert_eq!(out.len(), n, "{name} stream {k} length");
                assert_eq!(st.matches, matches[k], "{name} stream {k} matches");
                assert_eq!(st.literals, literals[k], "{name} stream {k} literals");
                assert_eq!(
                    raw.len() - st.consumed,
                    after[k],
                    "{name} stream {k} bytes after end-of-stream"
                );
                longest = longest.max(st.longest_match);
                far = far.max(st.longest_distance);
                zero += st.distance_zero_matches;
            }
            assert_eq!(longest, num(&row[t.col("Longest match")]), "{name}");
            assert_eq!(far, num(&row[t.col("Longest distance")]), "{name}");
            assert_eq!(zero, num(&row[t.col("Distance-0")]), "{name}");

            let d = &decoded.design;
            assert_eq!(d.commands.len(), n, "{name}");
            // At most one end-of-design record, always last; one
            // producer's HUS files carry none.
            let ends = d
                .commands
                .iter()
                .filter(|c| matches!(c, Command::End))
                .count();
            assert!(ends <= 1, "{name}");
            if ends == 1 {
                assert!(matches!(d.commands.last(), Some(Command::End)), "{name}");
            } else {
                assert!(name.ends_with("-me.hus"), "{name}: no end record");
                no_end += 1;
            }
            assert_eq!(
                d.counts().color_changes + 1,
                f.palette.len(),
                "{name} colour blocks"
            );
            if is_vip {
                assert!(f.vip_name.is_some(), "{name}: VIP name");
            }
            let e = d.extents();
            assert_eq!(
                (e.max_x, e.max_y, e.min_x, e.min_y),
                (
                    f.extents.0 as i32,
                    f.extents.1 as i32,
                    f.extents.2 as i32,
                    f.extents.3 as i32
                ),
                "{name} extents"
            );

            // read → write → read: producer-shaped (literal-only) GL.
            let opts = hus::HusEncodeOptions::default()
                .with_palette(f.palette.clone())
                .with_label(if is_vip {
                    f.vip_name.clone().unwrap_or_default()
                } else {
                    f.label.clone().unwrap_or_default()
                })
                .with_magic((!is_vip).then_some(f.magic));
            let bytes = if is_vip {
                hus::encode_vip(d, &opts).unwrap()
            } else {
                hus::encode(d, &opts).unwrap()
            };
            let again = hus::decode(&bytes).unwrap();
            // The writer always closes the streams with the end record.
            let mut want = d.commands.clone();
            if ends == 0 {
                want.push(Command::End);
            }
            assert_eq!(again.design.commands, want, "{name} round-trip");
            assert_eq!(again.file.magic, f.magic, "{name}");
            assert_eq!(again.file.extents, f.extents, "{name}");
            assert_eq!(again.file.palette, f.palette, "{name}");
            assert_eq!(again.file.label, f.label, "{name}");
            assert_eq!(again.file.vip_name, f.vip_name, "{name}");

            // … and through the match-bearing encoder.
            let mut lz = again.file.clone();
            let m = lz.stitch_count as usize;
            for raw in [&mut lz.attributes, &mut lz.x_deltas, &mut lz.y_deltas] {
                let (plain, _) = gl::decompress(raw, m).unwrap();
                *raw = gl::compress_lz(&plain, gl::DEFAULT_WINDOW).unwrap();
            }
            assert_eq!(hus::decode_design(&lz).unwrap().commands, want);
            checked += 1;
        }
    }
    if checked > 0 {
        assert_eq!(checked, 33, "expected all 33 staged HUS/VIP fixtures");
        assert_eq!(no_end, 5, "the five Wings XP .hus files");
    }
}

// ───────────────────────────── PES / PEC ─────────────────────────────

fn pec_block_of(data: &[u8]) -> (usize, pec::PecBlock) {
    if pes::probe(data) {
        let at = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        (at, pes::decode(data).unwrap().pec)
    } else {
        (8, pec::decode(data).unwrap())
    }
}

/// The `LA:` label field as the notes print it: the 16 raw bytes with
/// trailing whitespace dropped (one producer writes a lone tab before
/// the space fill), non-ASCII bytes as `\xNN`.
fn label_field(data: &[u8], at: usize) -> String {
    let raw = &data[at + 3..at + 19];
    let end = raw
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    raw[..end]
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                (b as char).to_string()
            } else {
                format!("\\x{b:02x}")
            }
        })
        .collect()
}

/// Model equality modulo the colour-change index byte, which the PEC
/// writer regenerates.
fn same_stitches(a: &Design, b: &Design) -> bool {
    a.commands.len() == b.commands.len()
        && a.commands
            .iter()
            .zip(&b.commands)
            .all(|(x, y)| match (x, y) {
                (
                    Command::ColorChange { dx, dy, .. },
                    Command::ColorChange { dx: u, dy: v, .. },
                ) => (dx, dy) == (u, v),
                _ => x == y,
            })
}

/// Every PES / PEC fixture: version string, PEC offset, section-1
/// `+31..+33` and `+40..+45` and the `LA:` label exactly as the notes
/// state; the stitch list decodes to its `0xFF` end, its integrated
/// extents fit the section-2 size, and the colour changes match the
/// colour table. Then read → write → read (standalone `.pec` and
/// `#PES0001`) with identical stitches, the writers emitting the
/// pinned classic `20 FF 00` form, and a PE-Design-layout block
/// re-emitted with the form its version code selects.
#[test]
fn pes_pec_fixtures_decode_to_notes() {
    let mut checked = 0;
    let mut pe_design_layout = 0;
    let mut size_laws: BTreeMap<&str, usize> = BTreeMap::new();
    for dir in fixture_dirs("pes") {
        let t = notes_table(&dir, "Version string");
        for path in data_files(&dir) {
            let name = name_of(&path);
            let row = &t.rows[&name];
            let data = std::fs::read(&path).unwrap();
            assert_eq!(
                std::str::from_utf8(&data[..8]).unwrap(),
                unquote(&row[t.col("Version string")]),
                "{name}"
            );
            let (at, block) = pec_block_of(&data);
            assert_eq!(at, num(&row[t.col("PEC at")]), "{name} PEC offset");
            let want31: Vec<u8> = ["+31", "+32", "+33"]
                .iter()
                .flat_map(|c| hex_bytes(&row[t.col(c)]))
                .collect();
            assert_eq!(block.layout_bytes.to_vec(), want31, "{name} +31..+33");
            assert_eq!(&data[at + 31..at + 34], &want31[..], "{name}");
            assert_eq!(
                block.writer_mark.to_vec(),
                hex_bytes(&row[t.col("+40..+45")]),
                "{name} +40..+45"
            );
            assert_eq!(
                label_field(&data, at),
                unquote(&row[t.col("label")]),
                "{name} label"
            );

            // Stitch facts.
            let d = &block.design;
            assert!(matches!(d.commands.last(), Some(Command::End)), "{name}");
            assert_eq!(
                d.counts().color_changes + 1,
                block.palette.len(),
                "{name} colour blocks"
            );
            // Section-2 width/height is producer-dependent: the
            // extent span, the span + 1, a hoop-sized value covering
            // it, or zero. Never smaller than the design.
            let e = d.extents();
            let (w, h) = (block.width as i32, block.height as i32);
            assert!(
                (w == 0 && h == 0) || (w >= e.width() && h >= e.height()),
                "{name}: extents {}x{} exceed section-2 size {w}x{h}",
                e.width(),
                e.height(),
            );
            let law = if (w, h) == (0, 0) {
                "zero"
            } else if (w, h) == (e.width(), e.height()) {
                "span"
            } else if (w, h) == (e.width() + 1, e.height() + 1) {
                "span+1"
            } else {
                "hoop"
            };
            *size_laws.entry(law).or_insert(0) += 1;

            // read → write → read.
            let opts = PecEncodeOptions::default()
                .with_palette(block.palette.clone())
                .with_label(if block.label.is_ascii() && block.label.len() <= 16 {
                    block.label.clone()
                } else {
                    String::new()
                });
            for (kind, bytes) in [
                ("pec", pec::encode(d, &opts).unwrap()),
                ("pes", pes::encode(d, &opts).unwrap()),
            ] {
                let (wat, again) = pec_block_of(&bytes);
                assert!(same_stitches(&again.design, d), "{name} → {kind}");
                assert_eq!(again.palette, block.palette, "{name} → {kind}");
                assert_eq!(&bytes[wat + 31..wat + 34], &[0x20, 0xFF, 0x00], "{name}");
                assert_eq!(again.writer_mark, [0x20; 6], "{name} → {kind}");
            }

            // A PE-Design-layout file's +31..+33 form is the one its
            // version code selects.
            if pes::probe(&data) && block.has_pe_design_mark() {
                let code = std::str::from_utf8(&data[4..8]).unwrap();
                let layout = PecLayout::for_pes_version(code);
                let b = pec::encode_block(d, &opts.clone().with_layout(layout)).unwrap();
                assert_eq!(b[31..34], block.layout_bytes, "{name} layout form");
                pe_design_layout += 1;
            }
            assert!(
                PecLayout::from_bytes(block.layout_bytes).is_some(),
                "{name}"
            );
            checked += 1;
        }
    }
    if checked > 0 {
        assert_eq!(checked, 28, "expected all 28 staged PES/PEC fixtures");
        assert_eq!(pe_design_layout, 7, "six -brother-vN files plus Star.pes");
        // 19 write the span, 8 a hoop-sized value, 1 zero; no fixture
        // writes span + 1 (the private corpus's form).
        let want: BTreeMap<&str, usize> = [("hoop", 8), ("span", 19), ("zero", 1)].into();
        assert_eq!(size_laws, want, "PEC section-2 size laws");
    }
}

/// A design's standalone `.pec` and the PEC block its `.pes` sibling
/// embeds (same producer tag) describe the same design: the same
/// thread palette, the same colour changes and the same extent
/// magnitudes. (They are not record-identical: the writers route the
/// opening moves differently, and one converter rotates the PEC copy
/// by a quarter turn.)
#[test]
fn pec_and_pes_siblings_agree() {
    let mut pairs = 0;
    for dir in fixture_dirs("pes") {
        let files = data_files(&dir);
        for p in &files {
            let name = name_of(p);
            let Some(stem) = name
                .strip_suffix(".pec")
                .or_else(|| name.strip_suffix(".PEC"))
            else {
                continue;
            };
            let Some(sib) = files
                .iter()
                .find(|q| name_of(q).eq_ignore_ascii_case(&format!("{stem}.pes")))
            else {
                continue;
            };
            let a = pec_block_of(&std::fs::read(p).unwrap()).1;
            let b = pec_block_of(&std::fs::read(sib).unwrap()).1;
            assert_eq!(a.palette, b.palette, "{name}");
            assert_eq!(
                a.design.counts().color_changes,
                b.design.counts().color_changes,
                "{name}"
            );
            let (ea, eb) = (a.design.extents(), b.design.extents());
            let mut ma = [ea.width(), ea.height()];
            let mut mb = [eb.width(), eb.height()];
            ma.sort();
            mb.sort();
            assert_eq!(ma, mb, "{name} extent magnitudes");
            pairs += 1;
        }
    }
    if fixtures_root().is_some() {
        assert_eq!(pairs, 5, "five .pec/.pes sibling pairs");
    }
}

// ───────────────────────────── XXX ─────────────────────────────

/// Every XXX fixture: palette escape form and offset, colour count at
/// `0x27`, slot 0, the filler run, slot 20, the terminator at
/// escape + 91 and the trailer length exactly as the notes state (and
/// the paletteless file parsed as such). Then read → write → read
/// byte-for-byte, and for every zero-filler file the pinned palette
/// block re-emitted from the used slots alone equals the file's block.
#[test]
fn xxx_fixtures_decode_to_notes() {
    let mut checked = 0;
    let mut reemitted = 0;
    for dir in fixture_dirs("xxx") {
        let t = notes_table(&dir, "Escape");
        for path in data_files(&dir) {
            let name = name_of(&path);
            let row = &t.rows[&name];
            let data = std::fs::read(&path).unwrap();
            let f = xxx::parse(&data).unwrap();
            assert_eq!(
                f.color_count as usize,
                num(&row[t.col("Colour count")]),
                "{name} colour count"
            );
            assert_eq!(xxx::write(&f).unwrap(), data, "{name} byte round-trip");
            assert!(matches!(
                xxx::decode_design(&f),
                Err(oxideav_embroidery::Error::Unsupported { .. })
            ));
            checked += 1;

            if unquote(&row[t.col("Escape")]) == "none" {
                assert_eq!(f.palette, None, "{name} has no palette block");
                assert!(f.trailer.is_empty());
                assert_eq!(f.stitch_data.len(), data.len() - xxx::HEADER_LEN);
                continue;
            }
            let p = f.palette.as_ref().unwrap_or_else(|| panic!("{name}"));
            assert_eq!(
                p.escape.bytes().to_vec(),
                hex_bytes(&row[t.col("Escape")]),
                "{name}"
            );
            let at = num(&row[t.col("Escape at")]);
            assert_eq!(p.offset, at, "{name} escape offset");
            assert_eq!(f.stitch_data.len(), at - xxx::HEADER_LEN, "{name}");
            assert_eq!(
                data[at + 91..at + 96].to_vec(),
                hex_bytes(&row[t.col("Bytes at escape+91")]),
                "{name}"
            );
            assert_eq!(f.trailer.len(), num(&row[t.col("Bytes after")]), "{name}");
            assert_eq!(p.slots[0], rgb(&row[t.col("Slot 0")]), "{name} slot 0");
            assert_eq!(p.slots[20], rgb(&row[t.col("Slot 20")]), "{name} slot 20");
            let before = num(&row[t.col("Slots before")]);
            let filler = unquote(&row[t.col("Filler")]);
            if let Some((v, times)) = filler.split_once("` ×") {
                let fill = rgb(v);
                assert_eq!(num(times), 20 - before, "{name} filler run length");
                assert!(
                    p.slots[before..20].iter().all(|s| *s == fill),
                    "{name} filler {filler}"
                );
                assert_ne!(p.slots[before - 1], fill, "{name} run starts at {before}");
            } else {
                assert_eq!(before, 20, "{name}");
            }
            match f.used_slots() {
                Some(n) => {
                    assert_eq!(n, f.color_count as usize, "{name}");
                    assert_eq!(f.threads().len(), n);
                    assert_eq!(f.threads()[0].rgb, Some(p.slots[0]));
                }
                None => assert_eq!(f.color_count, 0, "{name}"),
            }

            // Zero-filler files: the writer's pinned block from the
            // used slots alone is the file's block, byte for byte.
            if let Some(n) = f.used_slots()
                && p.slots[n..].iter().all(|s| *s == [0, 0, 0])
            {
                let colors: Vec<[u8; 3]> = p.slots[..n].to_vec();
                let block = xxx::encode_palette_block(p.escape, &colors).unwrap();
                assert_eq!(block, data[at..at + 96], "{name} palette block");
                assert_eq!(p.escape, PaletteEscape::Form02, "{name}");
                reemitted += 1;
            }
        }
    }
    if checked > 0 {
        assert_eq!(checked, 19, "expected all 19 staged XXX fixtures");
        assert_eq!(reemitted, 10, "Wilcom + open-source-writer files");
    }
}
