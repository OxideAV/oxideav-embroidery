//! Brother PES — the design container around a PEC machine block.
//!
//! Per the workspace's staged documentation (`docs/embroidery/pes/`):
//! a `#PESxxxx` 8-byte version signature, a u32 LE pointer at offset
//! 8 to the embedded PEC block, a version-dependent design section
//! (vector object model), then the PEC block the machine executes.
//! Every `.pes` file carries a complete PEC block, so decoding reads
//! the pointer and hands off to [`crate::pec`]; the design section is
//! preserved as raw bytes.
//!
//! The PES design-section object model is documented upstream only in
//! implementation-derived (Grade-B) material, which this workspace
//! does not consult, so this module does not parse or author it.
//! [`encode`] writes a container-minimal `#PES0001` file: signature,
//! PEC pointer, the documented `00 00 00 00` design-section
//! terminator, then the PEC block — the machine-side content is
//! complete; editor-side vectors are absent.

use crate::pec::{self, PecBlock, PecEncodeOptions};
use crate::{Design, Error, Result};

/// Version codes recognised by Brother's own tooling, mapped to the
/// vendor release that writes them. Extracted as data in the staged
/// table `docs/embroidery/pes/tables/pes-version-codes.csv`.
pub const PES_VERSIONS: [(&str, &str); 30] = [
    ("0001", "1.0"),
    ("0020", "2.0"),
    ("0025", "2.5"),
    ("0030", "3.0"),
    ("0040", "4.0"),
    ("0050", "5.0"),
    ("0055", "5.5"),
    ("0056", "5.6"),
    ("0060", "6.0"),
    ("0070", "7.0"),
    ("0071", "7.1"),
    ("0080", "8.0"),
    ("0090", "9.0"),
    ("0091", "9.1"),
    ("0092", "9.2"),
    ("0093", "9.3"),
    ("0096", "9.6"),
    ("0097", "9.7"),
    ("0100", "10.0"),
    ("0101", "10.1"),
    ("0102", "10.2"),
    ("0103", "10.3"),
    ("0106", "10.6"),
    ("0107", "10.7"),
    ("0108", "10.8"),
    ("0110", "11.0"),
    ("0111", "11.1"),
    ("0112", "11.2"),
    ("0113", "11.3"),
    ("0114", "11.4"),
];

/// Maps a 4-character version code (e.g. `"0040"`) to the vendor
/// release string (e.g. `"4.0"`).
pub fn version_name(code: &str) -> Option<&'static str> {
    PES_VERSIONS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, v)| *v)
}

/// A decoded PES file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PesFile {
    /// The 4-character version code from the signature.
    pub version_code: String,
    /// The vendor release that writes this code, when recognised.
    pub version: Option<&'static str>,
    /// The raw, unparsed PES design section (bytes 12 .. PEC offset).
    pub design_section: Vec<u8>,
    /// The embedded PEC block (stitches, palette, thumbnails).
    pub pec: PecBlock,
}

/// Returns true when `data` starts with a `#PES` signature.
pub fn probe(data: &[u8]) -> bool {
    data.len() >= 12 && &data[..4] == b"#PES"
}

/// Decodes a PES file via its embedded PEC block.
pub fn decode(data: &[u8]) -> Result<PesFile> {
    if data.len() < 12 {
        return Err(Error::UnexpectedEof {
            context: "PES header",
        });
    }
    if &data[..4] != b"#PES" {
        return Err(Error::BadMagic { expected: "PES" });
    }
    let version_code = String::from_utf8_lossy(&data[4..8]).into_owned();
    let pec_offset = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    if pec_offset < 12 || pec_offset >= data.len() {
        return Err(Error::invalid(format!(
            "PES PEC-block offset {pec_offset} outside file of {} bytes",
            data.len()
        )));
    }
    let pec = pec::decode_block(&data[pec_offset..])?;
    Ok(PesFile {
        version: version_name(&version_code),
        version_code,
        design_section: data[12..pec_offset].to_vec(),
        pec,
    })
}

/// Encodes a container-minimal `#PES0001` file (see module docs).
pub fn encode(design: &Design, options: &PecEncodeOptions) -> Result<Vec<u8>> {
    let block = pec::encode_block(design, options)?;
    let mut out = Vec::with_capacity(16 + block.len());
    out.extend_from_slice(b"#PES0001");
    out.extend_from_slice(&16u32.to_le_bytes());
    // Documented design-section stop terminator; no objects precede it.
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    out.extend_from_slice(&block);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Design};

    fn sample() -> Design {
        Design {
            commands: vec![
                Command::Jump { dx: 50, dy: 50 },
                Command::Stitch { dx: 10, dy: 0 },
                Command::Stitch { dx: 0, dy: 10 },
                Command::ColorChange {
                    dx: 0,
                    dy: 0,
                    index: None,
                },
                Command::Stitch { dx: -10, dy: -10 },
                Command::End,
            ],
            ..Default::default()
        }
    }

    #[test]
    fn version_table() {
        assert_eq!(version_name("0001"), Some("1.0"));
        assert_eq!(version_name("0040"), Some("4.0"));
        assert_eq!(version_name("0090"), Some("9.0"));
        assert_eq!(version_name("0114"), Some("11.4"));
        assert_eq!(version_name("9999"), None);
        assert_eq!(PES_VERSIONS.len(), 30);
    }

    #[test]
    fn roundtrip() {
        let d = sample();
        let bytes = encode(&d, &PecEncodeOptions::default()).unwrap();
        assert!(probe(&bytes));
        let f = decode(&bytes).unwrap();
        assert_eq!(f.version_code, "0001");
        assert_eq!(f.version, Some("1.0"));
        assert_eq!(f.design_section, vec![0, 0, 0, 0]);
        assert_eq!(f.pec.design.counts(), d.counts());
        assert_eq!(f.pec.palette, vec![1, 2]);
    }

    #[test]
    fn arbitrary_bytes_before_pec_tolerated() {
        // The staged docs record that arbitrary bytes are tolerated
        // between the design section and the PEC block.
        let d = sample();
        let block = crate::pec::encode_block(&d, &PecEncodeOptions::default()).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"#PES0040");
        let padding = 100usize;
        bytes.extend_from_slice(&((12 + padding) as u32).to_le_bytes());
        bytes.extend_from_slice(&vec![0xAB; padding]);
        bytes.extend_from_slice(&block);
        let f = decode(&bytes).unwrap();
        assert_eq!(f.version, Some("4.0"));
        assert_eq!(f.design_section.len(), padding);
        assert_eq!(f.pec.design.counts(), d.counts());
    }

    #[test]
    fn bad_offset_rejected() {
        let mut bytes = b"#PES0001".to_vec();
        bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        bytes.extend_from_slice(&[0; 600]);
        assert!(matches!(decode(&bytes), Err(Error::Invalid { .. })));
    }

    #[test]
    fn bad_magic_rejected() {
        assert!(matches!(decode(&[0u8; 600]), Err(Error::BadMagic { .. })));
    }
}
