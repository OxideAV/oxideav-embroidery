//! oxideav-core framework integration (the default-on `registry`
//! feature).
//!
//! Embroidery files enter the framework the same way other
//! non-audio/video assets do: a **container demuxer** that emits the
//! design file as a single data-stream packet, and a **codec
//! decoder** that turns that packet into a resolution-independent
//! [`Frame::Vector`] rendering of the sewn path (one polyline path
//! per colour block, stroked with the thread colour when the format
//! stores one).
//!
//! Both halves follow the workspace's dual-API convention: the
//! registry path installs everything via [`register`], and the
//! direct factories [`make_demuxer`] / [`make_decoder`] build the
//! same objects without a registry.
//!
//! The typed API ([`crate::decode`], the per-format modules and
//! [`crate::Design`]) remains the primary surface; this module is a
//! bridge, not a replacement.

use std::collections::VecDeque;
use std::io::Read;

use oxideav_core::registry::{
    CodecInfo, CodecRegistry, ContainerRegistry, Decoder, Demuxer, ProbeData, ProbeScore, ReadSeek,
    MAX_PROBE_SCORE, PROBE_SCORE_EXTENSION,
};
use oxideav_core::vector::{
    Group, Node, Path, PathNode, Point, Rgba, Stroke, VectorFrame, ViewBox,
};
use oxideav_core::{
    CodecCapabilities, CodecId, CodecParameters, CodecResolver, Frame, MediaType, Packet,
    RuntimeContext, StreamInfo, TimeBase,
};

use crate::{exp, Command, Design, Format};

/// The registered container and codec name.
pub const NAME: &str = "embroidery";

/// File extensions routed to this crate's demuxer (decodable design
/// formats only; palette side-files and header-only formats are not
/// claimed).
pub const EXTENSIONS: [&str; 13] = [
    "dst", "pes", "pec", "phc", "phb", "phx", "jef", "ptn", "jef+", "jpx", "hus", "vip", "exp",
];

/// Registers the embroidery demuxer, its content probe, the
/// extension map and the vector decoder into `ctx`.
pub fn register(ctx: &mut RuntimeContext) {
    register_containers(&mut ctx.containers);
    register_codecs(&mut ctx.codecs);
}

fn register_containers(reg: &mut ContainerRegistry) {
    reg.register_demuxer(NAME, open_demuxer);
    reg.register_probe(NAME, probe_container);
    for ext in EXTENSIONS {
        reg.register_extension(ext, NAME);
    }
}

fn register_codecs(reg: &mut CodecRegistry) {
    let mut caps = CodecCapabilities::audio("oxideav-embroidery");
    caps.media_type = MediaType::Data;
    caps.decode = true;
    caps.intra_only = true;
    caps.lossless = true;
    reg.register(
        CodecInfo::new(CodecId::new(NAME))
            .capabilities(caps)
            .decoder(make_decoder)
            .payload_magics([&b"#PES"[..], b"#PEC", b"#PHC", b"#PHB", b"#PHX"]),
    );
}

/// Content probe for the container registry. Scores per the registry
/// convention: unambiguous magics score 100, the structural DST/JEF
/// probes score 75 with extension corroboration and 50 without, and
/// the headerless EXP is extension-only (25).
pub fn probe_container(probe: &ProbeData) -> ProbeScore {
    let ext_claimed = probe
        .ext
        .is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)));
    match crate::probe(probe.buf) {
        // VP3 is recognised but its stitch section is undecodable, so
        // the demuxer does not claim it.
        Some(Format::Vp3) => 0,
        // Signature formats with unambiguous magic bytes.
        Some(
            Format::Pes
            | Format::Pec
            | Format::Phc
            | Format::Phb
            | Format::Phx
            | Format::Hus
            | Format::Vip,
        ) => MAX_PROBE_SCORE,
        // DST and JEF probes are structural rather than magic-based.
        Some(Format::Dst | Format::Jef) => {
            if ext_claimed {
                75
            } else {
                50
            }
        }
        None => {
            // Headerless EXP: extension hint only.
            if probe.ext.is_some_and(|e| e.eq_ignore_ascii_case("exp")) {
                PROBE_SCORE_EXTENSION
            } else {
                0
            }
        }
    }
}

/// Decodes `data` as any supported design format, with the
/// documented EXP fallback for the headerless case: EXP is accepted
/// only when the whole buffer walks as EXP records and the resulting
/// geometry is design-plausible.
fn decode_any(data: &[u8]) -> crate::Result<(&'static str, Design)> {
    match crate::decode(data) {
        Ok((format, design)) => Ok((format_label(format), design)),
        Err(crate::Error::BadMagic { .. }) => {
            let design = exp::decode(data)?;
            let e = design.extents();
            let plausible = !design.commands.is_empty()
                && e.width() < 100_000
                && e.height() < 100_000
                && (e.width() > 0 || e.height() > 0)
                && design.counts().stitches > 0;
            if plausible {
                Ok(("exp", design))
            } else {
                Err(crate::Error::BadMagic {
                    expected: "known embroidery",
                })
            }
        }
        Err(e) => Err(e),
    }
}

fn format_label(format: Format) -> &'static str {
    match format {
        Format::Dst => "dst",
        Format::Pes => "pes",
        Format::Pec => "pec",
        Format::Phc => "phc",
        Format::Phb => "phb",
        Format::Phx => "phx",
        Format::Jef => "jef",
        Format::Hus => "hus",
        Format::Vip => "vip",
        Format::Vp3 => "vp3",
    }
}

fn invalid(e: crate::Error) -> oxideav_core::Error {
    oxideav_core::Error::InvalidData(e.to_string())
}

// ───────────────────────── demuxer ─────────────────────────

/// Demuxer for embroidery design files: one data stream, one packet
/// carrying the whole file, with the design's headline numbers
/// surfaced as container metadata.
pub struct EmbroideryDemuxer {
    streams: Vec<StreamInfo>,
    metadata: Vec<(String, String)>,
    payload: Option<Vec<u8>>,
}

impl EmbroideryDemuxer {
    /// Builds the demuxer from the full file contents.
    pub fn from_bytes(data: Vec<u8>) -> oxideav_core::Result<Self> {
        let (label, design) = decode_any(&data).map_err(invalid)?;
        let c = design.counts();
        let e = design.extents();
        let mut metadata = vec![
            ("embroidery_format".to_string(), label.to_string()),
            ("stitches".to_string(), c.stitches.to_string()),
            ("jumps".to_string(), c.jumps.to_string()),
            ("trims".to_string(), c.trims.to_string()),
            (
                "color_blocks".to_string(),
                design.color_block_count().to_string(),
            ),
            ("width_0_1mm".to_string(), e.width().to_string()),
            ("height_0_1mm".to_string(), e.height().to_string()),
        ];
        if let Some(l) = &design.label {
            metadata.push(("title".to_string(), l.clone()));
        }
        let params = CodecParameters::data(CodecId::new(NAME));
        let streams = vec![StreamInfo {
            index: 0,
            time_base: TimeBase::new(1, 1),
            duration: None,
            start_time: None,
            params,
        }];
        Ok(Self {
            streams,
            metadata,
            payload: Some(data),
        })
    }
}

impl Demuxer for EmbroideryDemuxer {
    fn format_name(&self) -> &str {
        NAME
    }

    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }

    fn next_packet(&mut self) -> oxideav_core::Result<Packet> {
        match self.payload.take() {
            Some(data) => Ok(Packet::new(0, TimeBase::new(1, 1), data)),
            None => Err(oxideav_core::Error::Eof),
        }
    }

    fn metadata(&self) -> &[(String, String)] {
        &self.metadata
    }
}

/// Direct factory: opens `input` as an embroidery design file.
pub fn make_demuxer(mut input: Box<dyn ReadSeek>) -> oxideav_core::Result<Box<dyn Demuxer>> {
    let mut data = Vec::new();
    input.read_to_end(&mut data)?;
    Ok(Box::new(EmbroideryDemuxer::from_bytes(data)?))
}

fn open_demuxer(
    input: Box<dyn ReadSeek>,
    _codecs: &dyn CodecResolver,
) -> oxideav_core::Result<Box<dyn Demuxer>> {
    make_demuxer(input)
}

// ───────────────────────── decoder ─────────────────────────

/// Decoder for the `embroidery` codec id: each packet (a whole
/// design file) decodes to one [`Frame::Vector`] rendering of the
/// sewn path.
pub struct EmbroideryDecoder {
    id: CodecId,
    pending: VecDeque<Frame>,
    flushed: bool,
}

impl EmbroideryDecoder {
    /// A fresh decoder.
    pub fn new() -> Self {
        Self {
            id: CodecId::new(NAME),
            pending: VecDeque::new(),
            flushed: false,
        }
    }
}

impl Default for EmbroideryDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder for EmbroideryDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.id
    }

    fn send_packet(&mut self, packet: &Packet) -> oxideav_core::Result<()> {
        let (_, design) = decode_any(&packet.data).map_err(invalid)?;
        let mut frame = design_to_vector(&design);
        frame.pts = packet.pts;
        self.pending.push_back(Frame::Vector(frame));
        Ok(())
    }

    fn receive_frame(&mut self) -> oxideav_core::Result<Frame> {
        match self.pending.pop_front() {
            Some(f) => Ok(f),
            None if self.flushed => Err(oxideav_core::Error::Eof),
            None => Err(oxideav_core::Error::NeedMore),
        }
    }

    fn flush(&mut self) -> oxideav_core::Result<()> {
        self.flushed = true;
        Ok(())
    }
}

/// Direct factory matching the registry's `DecoderFactory` shape.
pub fn make_decoder(_params: &CodecParameters) -> oxideav_core::Result<Box<dyn Decoder>> {
    Ok(Box::new(EmbroideryDecoder::new()))
}

// ───────────────────────── vector rendering ─────────────────────────

/// Renders a design as a vector frame: one stroked polyline path per
/// colour block, in 0.1 mm user units, with the model's y-up
/// convention flipped to the vector module's y-down device space.
/// The stroke colour is the block's thread RGB when the format
/// stores one, black otherwise; stroke width is one design unit.
pub fn design_to_vector(design: &Design) -> VectorFrame {
    let e = design.extents();
    let width = (e.width() + 1) as f32;
    let height = (e.height() + 1) as f32;
    let to_point = |x: i32, y: i32| Point::new((x - e.min_x) as f32, (e.max_y - y) as f32);

    let mut root = Group::default();
    let mut block = 0usize;
    let (mut x, mut y) = (0i32, 0i32);
    let mut path = Path::new();
    let mut pen_down = false;

    let flush = |path: &mut Path, block: usize, root: &mut Group, design: &Design| {
        if path.commands.is_empty() {
            return;
        }
        let rgb = design
            .threads
            .get(block)
            .and_then(|t| t.rgb)
            .unwrap_or([0, 0, 0]);
        let color = Rgba {
            r: rgb[0],
            g: rgb[1],
            b: rgb[2],
            a: 255,
        };
        let node = PathNode::new(std::mem::take(path)).with_stroke(Stroke::solid(1.0, color));
        root.children.push(Node::Path(node));
    };

    for c in &design.commands {
        let (dx, dy) = c.delta();
        match c {
            Command::Stitch { .. } => {
                if !pen_down {
                    path.move_to(to_point(x, y));
                    pen_down = true;
                }
                x += dx;
                y += dy;
                path.line_to(to_point(x, y));
            }
            Command::Jump { .. } | Command::Trim { .. } => {
                x += dx;
                y += dy;
                pen_down = false;
            }
            Command::ColorChange { .. } => {
                flush(&mut path, block, &mut root, design);
                block += 1;
                x += dx;
                y += dy;
                pen_down = false;
            }
            Command::Stop | Command::End => {}
        }
    }
    flush(&mut path, block, &mut root, design);

    VectorFrame::new(width, height)
        .with_view_box(ViewBox::new(0.0, 0.0, width, height))
        .with_root(root)
}
