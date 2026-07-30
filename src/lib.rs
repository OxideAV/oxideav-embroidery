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

pub mod dst;
pub mod model;
pub mod pec;

pub use model::{Command, Counts, Design, Extents, Thread};

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
