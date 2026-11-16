# EXP trim fixtures

One small design (ten stitch blocks on a line, a trim between
blocks) made solely to show what a trim looks like in Melco EXP,
contributed for this purpose by the reporter of the `80 80 07 00`
issue, with permission to include it in this repository.

| File | Written by | SHA-256 |
| ---- | ---------- | ------- |
| `EXP_trim_A.exp` | a commercial embroidery suite | `629c84ead2aefe0666c74cbbc58eb10e6765f9eb015cb1e478b0ce1810f0f7f6` |
| `EXP_trim_A.pec` | the same suite | `32a7ee9fe417b4b0e4982537dcdd7de9883a790e5463eea56141934a79fb969f` |
| `EXP_trim_B.exp` | an open-source editor | `e671ab241a072b02a1ca294d595a1273c64422f59888949c41aa314eaac7bd6e` |
| `EXP_trim_B.pec` | the same editor | `2dad57623e853a010b549d4d1ba8f8ad3c0d52babdadcf7881949094bf6d207a` |

Per the contributor, the design was cross-checked in a second
commercial suite, and the EXP versions were stitched on a machine
that reads EXP natively: the trims occurred.

What the files show (asserted by `tests/exp_trim.rs`):

- `80 80` occurs 11 times in A and 10 times in B, always with the
  payload `07 00`, and nowhere with any other payload.
- Each record sits at the needle position where the PEC rendering
  starts a trim-flagged move (10 in each PEC file); the travel to the
  next block follows as `80 04` jumps (A: two jumps and a final plain
  stitch record; B: two jumps, then a zero-length stitch).
- The payload is not a displacement: with it ignored, the 271 needle
  penetrations of A and B are identical, equal those of the PEC
  renderings, and A (which travels home at the end) finishes on
  `(0, 0)` exactly.
- A's 11th record follows the last stitch, before the travel home;
  neither B nor the PEC renderings carry a counterpart.
