//! Compucon / Singer XXX — header, palette block and raw sections.
//!
//! Per the workspace's staged documentation (`docs/embroidery/xxx/`,
//! read out of real files; no third-party description exists): a
//! fixed-size **256-byte header**, largely zero-filled, with the
//! colour count at `0x27`, followed by stitch data from `0x100`.
//!
//! **Palette block** (pinned over 19 redistributable files from five
//! producers plus the private corpus): the escape `7F 7F 0x 14`
//! (`x` = `02` or `03`, producer-dependent), three `00` bytes, then
//! **exactly 21 fixed four-byte slots** `R G B 00` at escape + 7 + 4k
//! (slot 0 = first thread, then thread order), then the terminator
//! `FF FF FF 00 01` at the **fixed offset escape + 91**. Some producers
//! write further bytes after the terminator; one writes no palette
//! block at all. The slots are read **by position** — unused slots
//! hold producer-dependent filler (zeros, one repeated value, a stock
//! palette, `FF FF FF` in slot 20) and a genuine white thread can sit
//! in any slot, so neither a first-non-zero nor a sentinel scan is
//! sound. The used slots are bounded by the colour count at `0x27`;
//! when that byte is 0 (one producer family writes 0) the bound comes
//! from the stitch stream's colour changes, which this module cannot
//! read (see below), so [`XxxFile::used_slots`] reports `None`.
//!
//! The stitch records between `0x100` and the palette block are
//! *consistent with* 2-byte signed delta pairs plus `0x7F` escape
//! forms, but their control vocabulary is not established upstream,
//! so the body is surfaced raw and [`decode_design`] returns
//! [`Error::Unsupported`] until the docs collaborator stages it.
//!
//! [`write()`] re-serialises a parsed file (header, raw body, palette
//! block rebuilt from its slots, trailing bytes) byte-for-byte, and
//! [`encode_palette_block`] emits the pinned 96-byte block.
//!
//! XXX has **no magic/signature** — the header opens with zeros —
//! so it cannot be probed from content and never appears in the
//! crate-level dispatch. Use [`parse`] directly when the extension
//! says `.xxx` (the same arrangement as headerless Melco EXP).

use crate::model::Thread;
use crate::{Design, Error, Result};

/// The fixed header length.
pub const HEADER_LEN: usize = 0x100;

/// Offset of the colour-count byte inside the header.
pub const COLOR_COUNT_OFFSET: usize = 0x27;

/// Number of fixed palette slots.
pub const PALETTE_SLOTS: usize = 21;

/// Offset of slot 0 from the palette escape.
pub const SLOT0_OFFSET: usize = 7;

/// Offset of the terminator from the palette escape.
pub const TERMINATOR_OFFSET: usize = 91;

/// The palette terminator.
pub const TERMINATOR: [u8; 5] = [0xFF, 0xFF, 0xFF, 0x00, 0x01];

/// Total palette-block length, escape through terminator.
pub const PALETTE_BLOCK_LEN: usize = TERMINATOR_OFFSET + TERMINATOR.len();

/// The palette escape's third byte. Both documented forms carry the
/// identical 21-slot layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PaletteEscape {
    /// `7F 7F 02 14` — the form written with the colour count stored
    /// at `0x27` and zero filler.
    #[default]
    Form02,
    /// `7F 7F 03 14` — the form whose producers (in the staged set)
    /// store 0 or the colour count at `0x27`.
    Form03,
}

impl PaletteEscape {
    /// The four escape bytes.
    pub fn bytes(self) -> [u8; 4] {
        match self {
            PaletteEscape::Form02 => [0x7F, 0x7F, 0x02, 0x14],
            PaletteEscape::Form03 => [0x7F, 0x7F, 0x03, 0x14],
        }
    }

    fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x02 => Some(PaletteEscape::Form02),
            0x03 => Some(PaletteEscape::Form03),
            _ => None,
        }
    }
}

/// The 21-slot palette block.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct XxxPalette {
    /// Which escape form introduced the block.
    pub escape: PaletteEscape,
    /// Byte offset of the escape within the file.
    pub offset: usize,
    /// All 21 slots, by position (`R, G, B`), filler included.
    pub slots: [[u8; 3]; PALETTE_SLOTS],
}

impl XxxPalette {
    /// A palette block with `colors` in the leading slots and zero
    /// filler. `offset` is left 0 (set by [`parse`] on read).
    pub fn new(escape: PaletteEscape, colors: &[[u8; 3]]) -> Result<Self> {
        if colors.len() > PALETTE_SLOTS {
            return Err(Error::OutOfRange {
                field: "XXX palette (21 slots)",
            });
        }
        let mut slots = [[0u8; 3]; PALETTE_SLOTS];
        slots[..colors.len()].copy_from_slice(colors);
        Ok(Self {
            escape,
            offset: 0,
            slots,
        })
    }

    /// Set `escape`.
    pub fn with_escape(mut self, escape: PaletteEscape) -> Self {
        self.escape = escape;
        self
    }

    /// Set `slots`.
    pub fn with_slots(mut self, slots: [[u8; 3]; PALETTE_SLOTS]) -> Self {
        self.slots = slots;
        self
    }

    /// The pinned 96-byte block bytes for these slots.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PALETTE_BLOCK_LEN);
        out.extend_from_slice(&self.escape.bytes());
        out.extend_from_slice(&[0, 0, 0]);
        for s in &self.slots {
            out.extend_from_slice(s);
            out.push(0);
        }
        out.extend_from_slice(&TERMINATOR);
        debug_assert_eq!(out.len(), PALETTE_BLOCK_LEN);
        out
    }
}

/// A parsed XXX file: the documented header facts, the palette block
/// when present, and the raw sections everything else lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct XxxFile {
    /// Colour count (header byte `0x27`; 0 in one producer family).
    pub color_count: u8,
    /// The raw 256-byte header (fields beyond the colour count are
    /// unmapped upstream).
    pub header: Vec<u8>,
    /// Raw stitch data from `0x100` up to the palette escape (to end
    /// of file when there is no palette block).
    pub stitch_data: Vec<u8>,
    /// The palette block; `None` for producers that write none.
    pub palette: Option<XxxPalette>,
    /// Bytes after the palette terminator (empty for most producers;
    /// one family writes 1,777 bytes there).
    pub trailer: Vec<u8>,
}

impl XxxFile {
    /// How many leading slots hold the design's threads, when the
    /// header says: `min(colour count, 21)` for a non-zero `0x27`,
    /// `None` when `0x27` is 0 or there is no palette block.
    pub fn used_slots(&self) -> Option<usize> {
        match (&self.palette, self.color_count) {
            (Some(_), n) if n > 0 => Some((n as usize).min(PALETTE_SLOTS)),
            _ => None,
        }
    }

    /// The design's threads (RGB from the used slots, in thread
    /// order). Empty when [`XxxFile::used_slots`] is `None`.
    pub fn threads(&self) -> Vec<Thread> {
        match (&self.palette, self.used_slots()) {
            (Some(p), Some(n)) => p.slots[..n]
                .iter()
                .map(|&rgb| Thread::default().with_rgb(Some(rgb)))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// Locates the palette block: the first escape `7F 7F {02|03} 14` at
/// or after `0x100` that is followed by three zero bytes, has a zero
/// pad byte in every slot and the terminator at escape + 91.
fn find_palette(data: &[u8]) -> Option<usize> {
    if data.len() < HEADER_LEN + PALETTE_BLOCK_LEN {
        return None;
    }
    (HEADER_LEN..=data.len() - PALETTE_BLOCK_LEN).find(|&p| {
        data[p] == 0x7F
            && data[p + 1] == 0x7F
            && PaletteEscape::from_byte(data[p + 2]).is_some()
            && data[p + 3] == 0x14
            && data[p + 4..p + 7] == [0, 0, 0]
            && (0..PALETTE_SLOTS).all(|k| data[p + SLOT0_OFFSET + 4 * k + 3] == 0)
            && data[p + TERMINATOR_OFFSET..p + PALETTE_BLOCK_LEN] == TERMINATOR
    })
}

/// Parses an XXX file's documented structure.
///
/// There is no signature to verify, so this can only check the
/// structural minimum: the file must be at least one full header
/// long. Callers should gate on the `.xxx` extension.
pub fn parse(data: &[u8]) -> Result<XxxFile> {
    if data.len() < HEADER_LEN {
        return Err(Error::UnexpectedEof {
            context: "XXX header",
        });
    }
    let (body_end, palette, trailer) = match find_palette(data) {
        Some(p) => {
            let mut slots = [[0u8; 3]; PALETTE_SLOTS];
            for (k, slot) in slots.iter_mut().enumerate() {
                let o = p + SLOT0_OFFSET + 4 * k;
                slot.copy_from_slice(&data[o..o + 3]);
            }
            let escape = PaletteEscape::from_byte(data[p + 2]).unwrap_or_default();
            (
                p,
                Some(XxxPalette {
                    escape,
                    offset: p,
                    slots,
                }),
                data[p + PALETTE_BLOCK_LEN..].to_vec(),
            )
        }
        None => (data.len(), None, Vec::new()),
    };
    Ok(XxxFile {
        color_count: data[COLOR_COUNT_OFFSET],
        header: data[..HEADER_LEN].to_vec(),
        stitch_data: data[HEADER_LEN..body_end].to_vec(),
        palette,
        trailer,
    })
}

/// Re-serialises a parsed file: header, raw stitch body, the palette
/// block rebuilt from its slots (pad bytes and terminator as pinned),
/// then the trailing bytes. A file from [`parse`] comes back
/// byte-for-byte; the colour count is written into the header at
/// `0x27`.
pub fn write(file: &XxxFile) -> Result<Vec<u8>> {
    if file.header.len() != HEADER_LEN {
        return Err(Error::invalid(format!(
            "XXX header is {} bytes, not {HEADER_LEN}",
            file.header.len()
        )));
    }
    let mut out = file.header.clone();
    out[COLOR_COUNT_OFFSET] = file.color_count;
    out.extend_from_slice(&file.stitch_data);
    if let Some(p) = &file.palette {
        out.extend_from_slice(&p.to_bytes());
    }
    out.extend_from_slice(&file.trailer);
    Ok(out)
}

/// Emits the pinned 96-byte palette block for `colors` (at most 21):
/// escape, three zero bytes, 21 `R G B 00` slots (unused slots
/// zero-filled), terminator `FF FF FF 00 01`.
pub fn encode_palette_block(escape: PaletteEscape, colors: &[[u8; 3]]) -> Result<Vec<u8>> {
    Ok(XxxPalette::new(escape, colors)?.to_bytes())
}

/// Stitch decoding — blocked on the unestablished record control
/// vocabulary; always returns [`Error::Unsupported`].
pub fn decode_design(_file: &XxxFile) -> Result<Design> {
    Err(Error::Unsupported {
        what: "XXX stitch-record control vocabulary is not established by the staged documentation",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic() -> Vec<u8> {
        let mut out = vec![0u8; HEADER_LEN];
        out[COLOR_COUNT_OFFSET] = 5;
        // Delta-pair-shaped bytes per the staged observation; their
        // meaning is not decoded, so they are opaque payload here.
        out.extend_from_slice(&[0x02, 0x05, 0xFE, 0xFB, 0x02, 0x05]);
        out
    }

    #[test]
    fn parse_synthetic() {
        let bytes = synthetic();
        let f = parse(&bytes).unwrap();
        assert_eq!(f.color_count, 5);
        assert_eq!(f.header.len(), HEADER_LEN);
        assert_eq!(f.stitch_data, vec![0x02, 0x05, 0xFE, 0xFB, 0x02, 0x05]);
        assert_eq!(f.palette, None);
        assert!(f.trailer.is_empty());
        assert_eq!(f.used_slots(), None);
        assert!(matches!(decode_design(&f), Err(Error::Unsupported { .. })));
        assert_eq!(write(&f).unwrap(), bytes);
    }

    #[test]
    fn header_only_file_parses_with_empty_stitch_data() {
        let f = parse(&vec![0u8; HEADER_LEN]).unwrap();
        assert_eq!(f.color_count, 0);
        assert!(f.stitch_data.is_empty());
    }

    #[test]
    fn short_file_rejected() {
        assert!(matches!(
            parse(&[0u8; HEADER_LEN - 1]),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn palette_block_layout_is_pinned() {
        let colors = [[0x5D, 0x72, 0x7E], [0xFF, 0xFF, 0xFF], [1, 2, 3]];
        let b = encode_palette_block(PaletteEscape::Form02, &colors).unwrap();
        assert_eq!(b.len(), 96);
        assert_eq!(&b[..7], &[0x7F, 0x7F, 0x02, 0x14, 0, 0, 0]);
        assert_eq!(&b[7..11], &[0x5D, 0x72, 0x7E, 0]);
        assert_eq!(&b[11..15], &[0xFF, 0xFF, 0xFF, 0]);
        assert_eq!(&b[15..19], &[1, 2, 3, 0]);
        assert!(b[19..91].iter().all(|&v| v == 0));
        assert_eq!(&b[91..], &TERMINATOR);
        let b3 = encode_palette_block(PaletteEscape::Form03, &colors).unwrap();
        assert_eq!(b3[2], 0x03);
        assert!(encode_palette_block(PaletteEscape::Form02, &[[0; 3]; 22]).is_err());
    }

    #[test]
    fn palette_read_positionally_with_white_slots() {
        // White threads in slot 1 and slot 20 must neither end the
        // palette early nor be mistaken for the terminator.
        let mut bytes = synthetic();
        let mut slots = [[0u8; 3]; PALETTE_SLOTS];
        slots[0] = [0x49, 0x79, 0x00];
        slots[1] = [0xFF, 0xFF, 0xFF];
        slots[2] = [0x10, 0x20, 0x30];
        slots[20] = [0xFF, 0xFF, 0xFF];
        let pal = XxxPalette::new(PaletteEscape::Form03, &[])
            .unwrap()
            .with_slots(slots);
        bytes[COLOR_COUNT_OFFSET] = 3;
        let body_end = bytes.len();
        bytes.extend_from_slice(&pal.to_bytes());
        bytes.extend_from_slice(&[0xAA; 17]);
        let f = parse(&bytes).unwrap();
        let p = f.palette.as_ref().unwrap();
        assert_eq!(p.offset, body_end);
        assert_eq!(p.escape, PaletteEscape::Form03);
        assert_eq!(p.slots, slots);
        assert_eq!(f.trailer, vec![0xAA; 17]);
        assert_eq!(f.used_slots(), Some(3));
        let rgbs: Vec<_> = f.threads().iter().map(|t| t.rgb.unwrap()).collect();
        assert_eq!(rgbs, vec![slots[0], slots[1], slots[2]]);
        assert_eq!(write(&f).unwrap(), bytes);
        // A zero colour count leaves the used-slot bound open.
        let mut z = f.clone();
        z.color_count = 0;
        assert_eq!(z.used_slots(), None);
        assert!(z.threads().is_empty());
    }

    #[test]
    fn nonzero_slot_pad_is_not_a_palette() {
        let mut bytes = synthetic();
        let mut block = encode_palette_block(PaletteEscape::Form02, &[[1, 2, 3]]).unwrap();
        block[SLOT0_OFFSET + 3] = 0x55;
        bytes.extend_from_slice(&block);
        let f = parse(&bytes).unwrap();
        assert_eq!(f.palette, None);
        assert_eq!(write(&f).unwrap(), bytes);
    }

    #[test]
    fn escape_without_terminator_is_not_a_palette() {
        let mut bytes = synthetic();
        bytes.extend_from_slice(&[0x7F, 0x7F, 0x02, 0x14, 0, 0, 0]);
        bytes.extend_from_slice(&[0u8; 120]);
        let f = parse(&bytes).unwrap();
        assert_eq!(f.palette, None);
        assert_eq!(f.stitch_data.len(), bytes.len() - HEADER_LEN);
    }
}
