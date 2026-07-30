# oxideav-embroidery

Pure-Rust machine-embroidery design formats: **Tajima DST** and the
**Brother PES / PEC / PHC / PHX** family, plus **Melco EXP** (with its
`.inf` colour companion) and **Janome JEF** — decode to a typed
stitch-design model and encode back.

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

## Format support

| Format | Decode | Encode | Notes |
| ------ | ------ | ------ | ----- |
| Tajima DST | ✅ | ✅ | Ternary 3-byte records, 512-byte ASCII header, trim-by-jump-run convention, tolerance for the stray-trailing-byte digitiser variant |
| Brother PEC | ✅ | ✅ | Standalone `.pec` and the embedded block; short/long-form axis codec, jump/trim flags, colour changes, 48×38 1-bit thumbnails (rendered on encode); ships the Brother 64-entry thread table as data |
| Brother PES | ✅ | ◐ | Decode via the embedded PEC block, design section preserved raw; all 30 recognised version codes mapped. Encode is container-minimal `#PES0001` (no vector object model — documented upstream only in implementation-derived sources this workspace does not use) |
| Brother PHC | ✅ | — | Validated container layout; PEC stream at the structural offset (`259 + 230 × colours` at standard geometry). No encode: the 163-byte design-record core is undocumented |
| Brother PHX | ◐ | — | Best-effort decode per the staged vendor-reader analysis; **no real `.phx` sample exists anywhere**, so the parser is unvalidated by design |
| Melco EXP | ✅ | ✅ | Headerless stitch list + `.inf` colour companion (decode + encode) |
| Janome JEF | ✅ | ✅ | Validated header, colour table, `80 01/02/10` escapes |
| Husqvarna HUS | ◐ | — | Header, extents, palette and the three raw compressed streams; stitch decompression undocumented |
| Husqvarna/Pfaff VP3 | ◐ | — | Signature probe + producer-string skim; stitch section undocumented |

`probe()` identifies every signature-bearing format; `decode()`
dispatches to the right module and returns the `Design`.

## Validation

The staged documentation was validated upstream against purchased
commercial design bundles carrying the same artwork in up to twelve
formats (see `docs/embroidery/provenance/` in the workspace). Those
designs are copyrighted and are **not** part of this repository; the
test suite instead re-applies the same methodology to self-synthesized
designs — encoding one model into every writable format and requiring
byte-level and stitch-list agreement on decode — alongside the staged
documentation's worked byte examples and validated header formulas,
which are locked in as unit tests.

Known unpinned details (encoder choices documented in the module docs,
awaiting further staged material): DST numeric-field padding, PEC
section-1 filler and two opaque section-2 fields, the JEF second
per-colour array, cross-format axis-sign conventions, and everything
listed under each module's "unvalidated" caveats.

## License

MIT — see [LICENSE](LICENSE).
