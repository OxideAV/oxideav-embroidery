//! Machine-embroidery design formats — Tajima DST and the Brother
//! PES / PEC / PHC / PHX family, plus Melco EXP and Janome JEF.
//!
//! Embroidery files drive computerised embroidery machines: a flat
//! list of relative needle moves plus machine commands (jump, trim,
//! colour change, stop, end), optionally wrapped in a design-side
//! container carrying thread colours, hoop extents, and metadata.
//!
//! This crate decodes those files to a typed stitch-design model
//! ([`Design`]) and encodes the model back to the machine formats.
//! All format truth comes from the workspace's staged clean-room
//! documentation under `docs/embroidery/`.
//!
//! # Example
//!
//! ```
//! use oxideav_embroidery::{dst, Command, Design, Format};
//!
//! // Three stitches, a colour change, one more stitch.
//! let design = Design {
//!     commands: vec![
//!         Command::Stitch { dx: 10, dy: 0 },
//!         Command::Stitch { dx: 0, dy: 10 },
//!         Command::Stitch { dx: -10, dy: 0 },
//!         Command::ColorChange { dx: 0, dy: 0, index: None },
//!         Command::Stitch { dx: 0, dy: -10 },
//!         Command::End,
//!     ],
//!     ..Default::default()
//! };
//!
//! // Encode as Tajima DST …
//! let bytes = dst::encode(&design, &dst::DstEncodeOptions::default())?;
//!
//! // … and decode any supported format back to the same model.
//! let (format, decoded) = oxideav_embroidery::decode(&bytes)?;
//! assert_eq!(format, Format::Dst);
//! assert_eq!(decoded.counts().stitches, 4);
//! assert_eq!(decoded.counts().color_changes, 1);
//! # Ok::<(), oxideav_embroidery::Error>(())
//! ```

pub mod col;
pub mod dst;
pub mod exp;
pub mod hus;
pub mod jef;
pub mod model;
pub mod pec;
pub mod pes;
pub mod phc;
pub mod phx;
pub mod vp3;
pub mod xxx;

pub use model::{Command, Counts, Design, Extents, Thread};

/// The file formats this crate recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// Tajima DST.
    Dst,
    /// Brother PES (design container with embedded PEC).
    Pes,
    /// Standalone Brother PEC.
    Pec,
    /// Brother PHC (machine-side design).
    Phc,
    /// Brother PHB (PHC's multi-design sibling).
    Phb,
    /// Brother PHX (current-generation machine-side design).
    Phx,
    /// Janome JEF.
    Jef,
    /// Husqvarna Viking HUS (header/metadata only).
    Hus,
    /// Husqvarna Viking VIP (HUS's successor; header/metadata only).
    Vip,
    /// Husqvarna Viking / Pfaff VP3 (signature/metadata only).
    Vp3,
}

/// Identifies the format of `data` from its signature.
///
/// Melco EXP is headerless and cannot be probed, so it never
/// appears here — use [`exp::decode`] directly when the extension
/// says `.exp`.
pub fn probe(data: &[u8]) -> Option<Format> {
    if pes::probe(data) {
        Some(Format::Pes)
    } else if pec::probe(data) {
        Some(Format::Pec)
    } else if phc::probe(data) {
        Some(Format::Phc)
    } else if phc::probe_phb(data) {
        Some(Format::Phb)
    } else if phx::probe(data) {
        Some(Format::Phx)
    } else if hus::probe(data) {
        Some(Format::Hus)
    } else if hus::probe_vip(data) {
        Some(Format::Vip)
    } else if vp3::probe(data) {
        Some(Format::Vp3)
    } else if dst::probe(data) {
        Some(Format::Dst)
    } else if jef::probe(data) {
        Some(Format::Jef)
    } else {
        None
    }
}

/// Probes `data` and decodes it to a [`Design`] with the detected
/// format. HUS is recognised but returns [`Error::Unsupported`]
/// (its stitch compression is undocumented).
pub fn decode(data: &[u8]) -> Result<(Format, Design)> {
    let format = probe(data).ok_or(Error::BadMagic {
        expected: "known embroidery",
    })?;
    let design = match format {
        Format::Dst => dst::decode(data)?.design,
        Format::Pes => pes::decode(data)?.pec.design,
        Format::Pec => pec::decode(data)?.design,
        Format::Phc => phc::decode(data)?.design,
        Format::Phb => phc::decode_phb(data)?.design,
        Format::Phx => phx::decode(data)?.design,
        Format::Jef => jef::decode(data)?.design,
        Format::Hus | Format::Vip => {
            return Err(Error::Unsupported {
                what: "HUS/VIP stitch streams use a compression scheme the staged documentation does not cover",
            });
        }
        Format::Vp3 => {
            return Err(Error::Unsupported {
                what: "VP3 stitch section is not covered by the staged documentation",
            });
        }
    };
    Ok((format, design))
}

/// Crate-level error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The input ended before a complete structure could be read.
    UnexpectedEof {
        /// What was being read.
        context: &'static str,
    },
    /// The file does not start with the format's magic/signature.
    BadMagic {
        /// The format whose magic was expected.
        expected: &'static str,
    },
    /// The input is structurally invalid for the format.
    Invalid {
        /// What is wrong.
        reason: String,
    },
    /// The capability is documented as existing but is not decodable
    /// from the staged documentation (e.g. the HUS compressed
    /// payload), or the model carries a command the target format
    /// cannot express.
    Unsupported {
        /// What is unsupported.
        what: &'static str,
    },
    /// A value cannot be represented by the target format (e.g. a
    /// label too long for a fixed-width header field).
    OutOfRange {
        /// The offending field.
        field: &'static str,
    },
}

impl Error {
    #[allow(dead_code)]
    pub(crate) fn invalid(reason: impl Into<String>) -> Self {
        Error::Invalid {
            reason: reason.into(),
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::UnexpectedEof { context } => {
                write!(f, "unexpected end of input while reading {context}")
            }
            Error::BadMagic { expected } => {
                write!(f, "bad magic: expected a {expected} signature")
            }
            Error::Invalid { reason } => write!(f, "invalid data: {reason}"),
            Error::Unsupported { what } => write!(f, "unsupported: {what}"),
            Error::OutOfRange { field } => write!(f, "value out of range for {field}"),
        }
    }
}

impl std::error::Error for Error {}

/// Convenience result alias.
pub type Result<T> = core::result::Result<T, Error>;
