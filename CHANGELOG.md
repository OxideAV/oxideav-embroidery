# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

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
- Husqvarna HUS header/metadata parsing (stitch streams blocked on
  undocumented compression).
- Husqvarna VIP header/metadata parsing: HUS's validated layout with
  its own magic; the 44 inserted bytes ahead of the block area are
  surfaced raw (`HusFile::unmapped`, also present on HUS parses).
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
