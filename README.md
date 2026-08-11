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
| Brother PEC | ✅ | ✅ | Standalone `.pec` and the embedded block; short/long-form axis codec, jump/trim flags, colour changes, 48×38 1-bit thumbnails (rendered on encode), corpus-invariant section constants, label-continuation area surfaced; ships the Brother 64-entry thread table as data |
| Brother PES | ✅ | ◐ | Decode via the embedded PEC block, design section preserved raw; all 30 recognised version codes mapped. Encode is container-minimal `#PES0001` (no vector object model — documented upstream only in implementation-derived sources this workspace does not use) |
| Brother PHC | ✅ | — | Validated container layout; PEC stream at the structural offset (`259 + 230 × colours` at standard geometry). No encode: the 163-byte design-record core is undocumented |
| Brother PHB | ✅ | — | PHC's multi-design sibling: second copyright string, body shifted +36, 9-byte design-record head (design count is a documented hint, not proven), PEC stream at `259 + 230 × colours + 45` |
| Brother PHX | ◐ | — | Best-effort decode per the staged vendor-reader analysis; **no real `.phx` sample exists anywhere**, so the parser is unvalidated by design |
| Melco EXP | ✅ | ✅ | Headerless stitch list + `.inf` colour companion (decode + encode, both corpus record-area conventions); corpus-pinned `80 04` jump escape |
| `.col` companion | ✅ | ✅ | Plain-text colour list (count line + `index,R,G,B` records); corpus-validated to carry the same RGBs as the `.inf` sibling |
| `.edr` companion | ✅ | ✅ | 4-byte RGB records closed by the `FF FF FF 00` sentinel; corpus-validated against `.col`/`.inf` siblings |
| Janome JEF | ✅ | ✅ | Validated header, colour table, `80 01/02/10` escapes; corpus-pinned extents order and second-array value |
| Janome PTN / JEF+ / JPX | ✅ | — | The JEF container family (format flag at 0x04: JEF/PTN = 10, JEF+/JPX = 1). PTN decodes as plain JEF; the flag-1 members decode their stitch stream with an empty colour table (colour layout unpinned) |
| Husqvarna HUS | ✅ | ✅ | Full stitch decode + encode: three ArchiveLib GL streams ([`gl`](src/gl.rs), documented bitstream-level upstream), five corpus-validated attribute bytes, inline 8-byte label. Corpus: all streams decode to exactly the declared count, record-identical to the EXP sibling |
| Husqvarna VIP | ✅ | ✅ | HUS's layout behind its own magic, plus the inserted UTF-16LE name record (parsed and re-encoded); compressed blocks byte-identical to the HUS sibling's, colour-table scale undocumented |
| Husqvarna/Pfaff VP3 | ◐ | — | Signature probe + producer-string skim; stitch section undocumented |
| Compucon/Singer XXX | ◐ | — | 256-byte header (colour count at `0x27`, corpus-validated) + raw sections; no signature so extension-gated, record vocabulary undocumented |
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

Known unpinned details (encoder choices documented in the module
docs, awaiting further staged material): the HUS filler bytes and
VIP's three constant u32 fields, PEC's design-derived section-2 tail
pair, the JEF second-array semantics, the GL match/offset rules
(documented from the vendor binary but unexercised by any real
stream), and everything listed under each module's "unvalidated"
caveats.

## License

MIT — see [LICENSE](LICENSE).
