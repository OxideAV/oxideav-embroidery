# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- Typed stitch-design model (`Design`): relative needle moves in
  0.1 mm units, jump/trim/colour-change/stop/end commands, thread
  palette, extents/counts/position helpers.
- Tajima DST decode + encode: ternary stitch records, ASCII header,
  trim-by-jump-run convention, stray-trailing-byte tolerance.
- Brother PEC decode + encode (standalone `.pec` and the embedded
  block): short/long-form axis codec, jump/trim flags, colour
  changes, per-colour 1-bit thumbnails, and the Brother 64-entry
  thread table as data.
- Brother PES decode (via the embedded PEC block, 30 version codes
  mapped) and container-minimal `#PES0001` encode.
- Brother PHC decode: validated container layout, palette,
  thumbnails, raw design record, PEC stream at the structural offset.
- Brother PHX best-effort decode (unvalidated — no real sample
  exists): header walk, RGB colour list, bitmap section, chunked
  body, PEC stream via `#VAR`.
- Melco EXP decode + encode plus the `.inf` colour companion.
- Janome JEF decode + encode: validated header, colour table,
  `80 01/02/10` escapes.
- Husqvarna HUS header/metadata parsing (stitch streams blocked on
  undocumented compression).
- Husqvarna/Pfaff VP3 signature probe + producer-string skim (stitch
  section undocumented).
- Format `probe()`/`decode()` dispatch and a cross-format agreement
  test suite over self-synthesized designs.
- Bootstrap scaffold: crate layout, CI shims, license, error stub.
