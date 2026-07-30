//! Machine-embroidery design formats — Tajima DST and the Brother
//! PES / PEC / PHC / PHX family.
//!
//! Embroidery files drive computerised embroidery machines: a flat
//! list of relative needle moves plus machine commands (jump, trim,
//! colour change, stop, end), optionally wrapped in a design-side
//! container carrying thread colours, hoop extents, and metadata.
//!
//! This crate decodes those files to a typed stitch-design model and
//! encodes the model back to the machine formats. All format truth
//! comes from the workspace's staged clean-room documentation.
//!
//! Bootstrap scaffold — the format surface lands in subsequent
//! commits.

/// Crate-level error type.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The requested capability has not been implemented yet.
    NotImplemented,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NotImplemented => f.write_str("not implemented yet"),
        }
    }
}

impl std::error::Error for Error {}
