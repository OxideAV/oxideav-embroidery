# oxideav-embroidery

Pure-Rust machine-embroidery design formats: **Tajima DST**, the
**Brother PES / PEC / PHC / PHB / PHX** family, **Melco EXP** (with
its `.inf` and `.col` colour companions), the **Janome JEF family**
(JEF / PTN / JEF+ / JPX) and **Husqvarna HUS / VIP** including their
compressed stitch streams — decode to a typed stitch-design model
and encode back — plus header/metadata depth for **VP3** and
**Compucon/Singer XXX**, exactly as far as the staged clean-room
documentation reaches.

Part of the [OxideAV](https://github.com/OxideAV) workspace: every
format is implemented clean-room from the workspace's staged
documentation — no external implementation source is consulted.

## The model

Every format decodes into, and encodes from, the same
[`Design`](src/model.rs): a flat command list of relative needle moves
in 0.1 mm design units (`Stitch` / `Jump` / `Trim` / `ColorChange` /
`Stop` / `End`), a thread palette (vendor index, RGB, catalogue tag,
name — whichever the format stores), a label, and derived extents /
counts / absolute-position helpers.

## Framework integration

With the default-on `registry` feature, the crate plugs into
`oxideav-core`: `register(ctx)` installs a container demuxer
(content probe + extension map for the thirteen decodable design
extensions) and a codec that decodes a design packet to a
`Frame::Vector` rendering — one stroked polyline path per colour
block, thread RGB when the format stores one. The dual-API
convention applies: `make_demuxer` / `make_decoder` direct factories
mirror the registry path, and `design_to_vector` is public. Building
with `--no-default-features` yields the dependency-free standalone
crate (its own CI leg keeps that path green).

## Format support

| Format | Decode | Encode | Notes |
| ------ | ------ | ------ | ----- |
| Tajima DST | ✅ | ✅ | Ternary 3-byte records, 512-byte ASCII header with corpus-pinned field padding, trim-by-jump-run convention, tolerance for the deviating-writer header forms and the stray-trailing-byte variant; `decode_header` exposed alone for the tape-family siblings (Barudan DSB, ZSK DSZ) that share the header over undocumented records |
| Brother PEC | ✅ | ✅ | Standalone `.pec` and the embedded block; short/long-form axis codec, jump/trim flags, colour changes, 48×38 1-bit thumbnails (rendered on encode), corpus-invariant section constants, label-continuation area surfaced; section-1 `+31..+33` and `+40..+45` surfaced raw (never validated), the writer picks `+31` by layout (`20 FF 00` classic — always for standalone `.pec` and `#PES0001` — or `FF FF 00` for PE-Design v5+ layouts); ships the Brother 64-entry thread table as data |
| Brother PES | ✅ | ◐ | Decode via the embedded PEC block, design section preserved raw; all 30 recognised version codes mapped. Encode is container-minimal `#PES0001` (no vector object model — documented upstream only in implementation-derived sources this workspace does not use) |
| Brother PHC | ✅ | — | Validated container layout; PEC stream at the structural offset (`259 + 230 × colours` at standard geometry). No encode: the 163-byte design-record core is undocumented |
| Brother PHB | ✅ | — | PHC's multi-design sibling: second copyright string, body shifted +36, 9-byte design-record head (design count is a documented hint, not proven), PEC stream at `259 + 230 × colours + 45` |
| Brother PHX | ◐ | — | Best-effort decode per the staged vendor-reader analysis; **no real `.phx` sample exists anywhere**, so the parser is unvalidated by design |
| Melco EXP | ✅ | ✅ | Headerless stitch list + `.inf` colour companion (decode + encode, both corpus record-area conventions); corpus-pinned `80 04` jump escape |
| `.col` companion | ✅ | ✅ | Plain-text colour list (count line + `index,R,G,B` records); corpus-validated to carry the same RGBs as the `.inf` sibling |
| `.edr` companion | ✅ | ✅ | 4-byte RGB records closed by the `FF FF FF 00` sentinel; corpus-validated against `.col`/`.inf` siblings |
| Janome JEF | ✅ | ✅ | Validated header, colour table, `80 01/02/10` escapes; corpus-pinned extents order and second-array value |
| Janome PTN / JEF+ / JPX | ✅ | — | The JEF container family (format flag at 0x04: JEF/PTN = 10, JEF+/JPX = 1). PTN decodes as plain JEF; the flag-1 members decode their stitch stream with an empty colour table (colour layout unpinned) |
| Husqvarna HUS | ✅ | ✅ | Full stitch decode + encode: three ArchiveLib GL streams ([`gl`](src/gl.rs), documented bitstream-level upstream; literal-only producer-shaped encoder plus a match-bearing `compress_lz` for the documented match layer), five corpus-validated attribute bytes, inline 8-byte label, both documented magics (`0x00C8AF5B` / `0x00C8FC5D`), the unused byte after end-of-stream and end-record-less streams tolerated. Corpus: all streams decode to exactly the declared count, record-identical to the EXP sibling; fixtures: 99 match-coded streams decode to every documented statistic |
| Husqvarna VIP | ✅ | ✅ | HUS's layout behind its own magic, plus the inserted UTF-16LE name record (kept raw; the name is read from its length-prefixed tail under any head, re-encoded in the corpus layout); compressed blocks byte-identical to the HUS sibling's, colour-table scale undocumented |
| Husqvarna/Pfaff VP3 | ◐ | — | Signature probe + producer-string skim; stitch section undocumented |
| Compucon/Singer XXX | ◐ | ◐ | 256-byte header (colour count at `0x27`) + the pinned palette block (escape `7F 7F 02/03 14`, 21 positional `R G B 00` slots, terminator `FF FF FF 00 01` at escape + 91, producer trailer) read and written exactly; parse → write is byte-exact. No signature so extension-gated; the stitch-record vocabulary is undocumented, so the body stays raw |
| Bernina ART | — | — | Extension-family recognition + version-from-extension hints only; the staged material is vendor statements about contents — no structure is documented anywhere |

`probe()` identifies every signature-bearing format; `decode()`
dispatches to the right module and returns the `Design`. EXP and XXX
carry no signature and are decoded/parsed by extension through their
modules (the framework demuxer applies a validated-walk fallback for
`.exp`); ART cannot even be that (no structural fact about it is
documented anywhere), so its module only recognises the extension
family.

## Validation

The staged documentation was validated upstream against purchased
commercial design bundles carrying the same artwork in up to
**62 formats** (see `docs/embroidery/provenance/` and
`docs/embroidery/corpus-map.md` in the workspace). Those designs are
copyrighted and are **not** part of this repository; the test suite
covers them two ways:

- **Synthetic round-trips** (always on): one model encoded into every
  writable format and required to agree byte-level and
  stitch-list-level on decode, alongside the staged documentation's
  worked byte examples and validated header formulas (the PHC/PHB
  stitch-offset rules, the GL bitstream's block structure, the `.edr`
  worked example …), locked in as unit tests.
- **Corpus-gated re-validation** (`tests/corpus.rs`, skips cleanly
  when the corpus is absent): every real HUS/VIP stream decodes to
  exactly its declared record count and matches the sibling EXP
  record-for-record (238k+ records), every signature-bearing corpus
  file probes and decodes, the JEF-family members agree with their
  `.jef` sibling, the colour companions agree RGB-for-RGB, and
  sibling formats of one design agree on extent-magnitude sets (the
  corpus contains rotated exports, so sets, not ordered pairs).
- **Redistributable fixture sweep** (`tests/fixtures_2026_09.rs`,
  skips cleanly when `docs/embroidery/fixtures-2026-09/` is absent):
  every staged file is checked against the facts its `notes.md` table
  states (read at runtime), then round-tripped through the writers.

  | Set | Files | Checked against the notes | Pass |
  | --- | ----: | ------------------------- | ---: |
  | HUS / VIP (Premier+, Wilcom, Wings XP, a test set) | 33 | magic, stitch count, colours, per-stream match / literal counts, longest match, longest distance, distance-0 matches, bytes after end-of-stream, integrated extents; read → write → read identical (literal-only and match-bearing GL) | 33 / 33 |
  | PES / PEC (PE-Design v1–v6 layouts, Premier+, Wilcom, Wings XP, converters, `#PES0070`, `#PES0110`) | 28 | version string, PEC offset, `+31..+33`, `+40..+45`, `LA:` label, colour blocks, section-2 size ≥ extents; read → write (`.pec` and `#PES0001`) → read identical; PE-Design-layout `+31` re-emitted from the version code (7 / 7) | 28 / 28 |
  | XXX (Premier+, Wilcom, open-source writer, Wings XP paletteless, a test set, a real design) | 19 | escape form + offset, colour count, slot 0, filler run, slot 20, terminator, trailer length; parse → write byte-exact (19 / 19); writer's block from the used slots alone equals the file's (10 / 10 zero-filler files) | 19 / 19 |
  | **Total** | **80** | | **80 / 80** |

- **Fuzzing** (`fuzz/`, libFuzzer): hostile raw GL bitstreams, a
  structure-aware GL round-trip property over both encoders and
  every window level, every public decode entry point plus the
  framework demuxer, and a structured HUS/VIP container target that
  always reaches the compressed layer.

The GL match/offset/window rules are exercised in-repo by the
match-bearing encoder's synthesized streams (every offset symbol at
both range edges, extreme match lengths, ring wraparound, all five
window levels) and validated on real streams by the fixture sweep
(match lengths 3…256, distances to 956, distance-0 runs).
Known unpinned details (encoder choices documented in the module
docs, awaiting further staged material): the HUS filler bytes and
the VIP inserted record's head, PEC's design-derived section-2 tail
pair and its producer-dependent width/height field (span, span + 1,
hoop size or zero), the JEF second-array semantics, and everything listed under
each module's "unvalidated" caveats.

## License

MIT — see [LICENSE](LICENSE).
