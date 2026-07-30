//! The typed stitch-design model.
//!
//! Every format in this crate decodes into — and encodes from — the
//! same model: a flat command list of relative needle moves plus
//! machine commands, a thread palette, and lightweight metadata.
//!
//! Units are **design units of 0.1 mm** on both axes; every staged
//! format in this crate (DST, PEC, EXP, JEF) uses that scale
//! natively. Axis convention: `+x` right, `+y` up, matching the DST
//! ternary table. Per-format axis orientation relative to this
//! convention is preserved as decoded; cross-format sign agreement
//! has not yet been pinned by the staged documentation (see the
//! crate README).

/// One machine command in a stitch design.
///
/// Deltas are relative needle moves in 0.1 mm design units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    /// A normal stitch: move the needle and sew.
    Stitch { dx: i32, dy: i32 },
    /// A jump: move the needle without sewing.
    Jump { dx: i32, dy: i32 },
    /// A trim: cut the thread, optionally moving. Formats without a
    /// distinct trim opcode (DST, EXP, JEF) express trims as jump
    /// conventions; PEC carries an explicit trim flag.
    Trim { dx: i32, dy: i32 },
    /// A colour change (which also stops the machine so the operator
    /// or changer can switch thread). `index` preserves the byte some
    /// formats (PEC) attach to the record.
    ColorChange { dx: i32, dy: i32, index: Option<u8> },
    /// An explicit machine stop that is not a colour change.
    Stop,
    /// End of design.
    End,
}

impl Command {
    /// The needle displacement this command carries.
    pub fn delta(&self) -> (i32, i32) {
        match *self {
            Command::Stitch { dx, dy }
            | Command::Jump { dx, dy }
            | Command::Trim { dx, dy }
            | Command::ColorChange { dx, dy, .. } => (dx, dy),
            Command::Stop | Command::End => (0, 0),
        }
    }
}

/// One thread in the design's palette. All fields are optional
/// because machine formats differ sharply in what they store: DST
/// and EXP store nothing, PEC stores vendor palette indices, PHX and
/// `.inf` companions store RGB, VP3 stores names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Thread {
    /// Vendor palette index as stored on the wire (e.g. a Brother
    /// PEC thread-table index, a Janome or Husqvarna code).
    pub palette_index: Option<u16>,
    /// RGB colour, when the format stores real colour data.
    pub rgb: Option<[u8; 3]>,
    /// Thread-catalogue tag (e.g. from an EXP `.inf` companion).
    pub catalog: Option<String>,
    /// Human-readable colour or thread name.
    pub name: Option<String>,
}

/// Signed bounding box of needle positions, in 0.1 mm design units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Extents {
    pub min_x: i32,
    pub max_x: i32,
    pub min_y: i32,
    pub max_y: i32,
}

impl Extents {
    /// Horizontal span (`max_x - min_x`).
    pub fn width(&self) -> i32 {
        self.max_x - self.min_x
    }
    /// Vertical span (`max_y - min_y`).
    pub fn height(&self) -> i32 {
        self.max_y - self.min_y
    }
}

/// Aggregate command counts for a design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub stitches: usize,
    pub jumps: usize,
    pub trims: usize,
    pub color_changes: usize,
    pub stops: usize,
}

/// A stitch design: the decode target and encode source for every
/// format in this crate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Design {
    /// The command list, in machine execution order. A well-formed
    /// design ends with [`Command::End`]; decoders append one when
    /// the format marks end-of-design.
    pub commands: Vec<Command>,
    /// Thread palette, one entry per colour block in sewing order
    /// (so `threads.len() == color_changes + 1` when known). Empty
    /// when the format stores no colour data.
    pub threads: Vec<Thread>,
    /// Design label, when the format stores one (DST `LA:`, PEC
    /// `LA:`).
    pub label: Option<String>,
}

impl Design {
    /// Accumulates every command's delta and returns the signed
    /// bounding box of all needle positions (the origin included).
    pub fn extents(&self) -> Extents {
        let (mut x, mut y) = (0i32, 0i32);
        let mut e = Extents::default();
        for c in &self.commands {
            let (dx, dy) = c.delta();
            x += dx;
            y += dy;
            e.min_x = e.min_x.min(x);
            e.max_x = e.max_x.max(x);
            e.min_y = e.min_y.min(y);
            e.max_y = e.max_y.max(y);
        }
        e
    }

    /// Aggregate command counts.
    pub fn counts(&self) -> Counts {
        let mut n = Counts::default();
        for c in &self.commands {
            match c {
                Command::Stitch { .. } => n.stitches += 1,
                Command::Jump { .. } => n.jumps += 1,
                Command::Trim { .. } => n.trims += 1,
                Command::ColorChange { .. } => n.color_changes += 1,
                Command::Stop => n.stops += 1,
                Command::End => {}
            }
        }
        n
    }

    /// Number of colour blocks: colour changes + 1 (a design with no
    /// colour change sews one colour). Zero for an empty design.
    pub fn color_block_count(&self) -> usize {
        if self.commands.is_empty() {
            0
        } else {
            self.counts().color_changes + 1
        }
    }

    /// The absolute needle positions after each command, starting
    /// from the origin. Useful for rendering and for cross-format
    /// comparison of the sewn path.
    pub fn positions(&self) -> Vec<(i32, i32)> {
        let (mut x, mut y) = (0i32, 0i32);
        self.commands
            .iter()
            .map(|c| {
                let (dx, dy) = c.delta();
                x += dx;
                y += dy;
                (x, y)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Design {
        Design {
            commands: vec![
                Command::Jump { dx: 10, dy: -5 },
                Command::Stitch { dx: 3, dy: 3 },
                Command::Stitch { dx: -20, dy: 0 },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::Trim { dx: 0, dy: 4 },
                Command::Stitch { dx: 1, dy: 1 },
                Command::End,
            ],
            threads: vec![Thread::default(), Thread::default()],
            label: Some("TEST".into()),
        }
    }

    #[test]
    fn counts() {
        let c = sample().counts();
        assert_eq!(c.stitches, 3);
        assert_eq!(c.jumps, 1);
        assert_eq!(c.trims, 1);
        assert_eq!(c.color_changes, 1);
        assert_eq!(c.stops, 0);
    }

    #[test]
    fn extents_track_all_positions() {
        let e = sample().extents();
        // Path x: 10, 13, -7, -7, -7, -6 ; y: -5, -2, -2, -2, 2, 3
        assert_eq!(e.min_x, -7);
        assert_eq!(e.max_x, 13);
        assert_eq!(e.min_y, -5);
        assert_eq!(e.max_y, 3);
        assert_eq!(e.width(), 20);
        assert_eq!(e.height(), 8);
    }

    #[test]
    fn color_blocks() {
        assert_eq!(sample().color_block_count(), 2);
        assert_eq!(Design::default().color_block_count(), 0);
    }

    #[test]
    fn positions_len() {
        let d = sample();
        assert_eq!(d.positions().len(), d.commands.len());
        assert_eq!(*d.positions().last().unwrap(), (-6, 3));
    }
}
