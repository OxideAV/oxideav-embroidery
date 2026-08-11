# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- ArchiveLib GL bitstream module (`gl`): full decoder for the
  block-structured static-Huffman LZSS scheme HUS/VIP compress with
  (matches, offsets, degenerate code sets, multi-block streams) plus
  a literal-only encoder matching the shape every real producer
  emits. General-purpose, not embroidery-specific.
- Husqvarna HUS/VIP **full stitch decode + encode** over the GL
  bitstream — previously header-only. Corpus-validated: all 33 real
  compressed streams decode to exactly the declared record count and
  the stitch stream matches the sibling EXP record-for-record.
- Corrected HUS header reading per the staged corpus findings: the
  inline 8-byte ASCII label at 0x20, and VIP's inserted name record
  (signature, three u32 fields, UTF-16LE design name) parsed rather
  than surfaced as opaque bytes.
- `.edr` colour-companion decode + encode (4-byte RGB records with
  the `FF FF FF 00` sentinel), cross-checked against `.col`/`.inf`
  siblings.
- JEF container family: `.ptn` decodes as plain JEF; the flag-1
  members (`.jef+`, `.jpx`) decode their stitch stream with an empty
  colour table (layout unpinned); date-stamp accessor; undocumented
  format flags now reject.
- oxideav-core framework integration behind a default-on `registry`
  feature: `register(ctx)` installs a container demuxer with content
  probe + extension map and a codec decoding designs to
  `Frame::Vector` (one stroked polyline per colour block);
  `make_demuxer`/`make_decoder` direct factories; public
  `design_to_vector`. The no-default-features build stays
  dependency-free.
- Corpus-gated integration tests (`tests/corpus.rs`) that re-run the
  staged-corpus validation against real purchased designs when
  present and skip cleanly otherwise.

### Changed

- Encoder choices pinned to the staged corpus findings: DST signed
  header fields are sign-first with space-padded magnitudes and
  `PD:******`; PEC writes the invariant section-1 `FF 00 06 26` and
  section-2 `0x31`/`0xF0FF` bytes and surfaces the 11-byte label
  continuation on decode; EXP jumps use the `80 04` escape; JEF's
  second per-colour array writes the corpus-uniform value 13.
- `.inf` decode accepts both record-area conventions observed in the
  corpus (counted from offset 16 or from offset 8).
- DST numeric parsing accepts the deviating writer's forms
  (zero-padded magnitudes, negative-signed zero, dashes).

- Typed stitch-design model (`Design`): relative needle moves in
  0.1 mm units, jump/trim/colour-change/stop/end commands, thread
  palette, extents/counts/position helpers.
- Tajima DST decode + encode: ternary stitch records, ASCII header,
  trim-by-jump-run convention, stray-trailing-byte tolerance.
- `dst::decode_header` as a standalone entry point for the
  corpus-documented tape-family siblings (Barudan DSB, ZSK DSZ) that
  carry DST's 512-byte header verbatim over differently-encoded,
  undocumented records.
- Brother PEC decode + encode (standalone `.pec` and the embedded
  block): short/long-form axis codec, jump/trim flags, colour
  changes, per-colour 1-bit thumbnails, and the Brother 64-entry
  thread table as data.
- Brother PES decode (via the embedded PEC block, 30 version codes
  mapped) and container-minimal `#PES0001` encode.
- Brother PHC decode: validated container layout, palette,
  thumbnails, raw design record, PEC stream at the structural offset.
- Brother PHB decode: PHC's multi-design sibling (second copyright
  string, +36 body shift, `259 + 230n + 45` stitch offset, design
  count surfaced as a documented-but-unproven hint).
- Brother PHX best-effort decode (unvalidated — no real sample
  exists): header walk, RGB colour list, bitmap section, chunked
  body, PEC stream via `#VAR`.
- Melco EXP decode + encode plus the `.inf` colour companion.
- `.col` plain-text colour-companion decode + encode (count line +
  `index,R,G,B` records; corpus-validated against `.inf` values).
- Janome JEF decode + encode: validated header, colour table,
  `80 01/02/10` escapes.
- Husqvarna HUS header/metadata parsing.
- Husqvarna VIP header/metadata parsing: HUS's validated layout with
  its own magic.
- Husqvarna/Pfaff VP3 signature probe + producer-string skim (stitch
  section undocumented).
- Compucon/Singer XXX header parsing: 256-byte header with the
  corpus-validated colour count at `0x27`, raw sections surfaced;
  extension-gated (no signature exists) and stitch decode blocked on
  the unestablished record vocabulary.
- Bernina ART extension-family recognition with
  version-from-extension hints (`.art42` … `.art90`); everything
  structural is a documented total gap upstream, so no content probe
  or parser exists yet.
- Format `probe()`/`decode()` dispatch and a cross-format agreement
  test suite over self-synthesized designs.
- Bootstrap scaffold: crate layout, CI shims, license, error stub.
