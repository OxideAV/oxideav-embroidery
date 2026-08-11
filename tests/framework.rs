//! oxideav-core framework integration (registry feature).
#![cfg(feature = "registry")]

use std::io::Cursor;

use oxideav_core::registry::{Decoder, ProbeData, MAX_PROBE_SCORE, PROBE_SCORE_EXTENSION};
use oxideav_core::{Error as CoreError, Frame, MediaType, NullCodecResolver, RuntimeContext};
use oxideav_embroidery::framework::{self, EmbroideryDecoder};
use oxideav_embroidery::{dst, hus, pec, Command, Design, Thread};

fn sample_design() -> Design {
    Design {
        commands: vec![
            Command::Jump { dx: 10, dy: 10 },
            Command::Stitch { dx: 20, dy: 0 },
            Command::Stitch { dx: 0, dy: 20 },
            Command::ColorChange {
                dx: 0,
                dy: 0,
                index: None,
            },
            Command::Stitch { dx: -20, dy: -20 },
            Command::End,
        ],
        threads: vec![
            Thread {
                rgb: Some([255, 0, 0]),
                ..Default::default()
            },
            Thread {
                rgb: Some([0, 0, 255]),
                ..Default::default()
            },
        ],
        label: None,
    }
}

fn registered() -> RuntimeContext {
    let mut ctx = RuntimeContext::new();
    oxideav_embroidery::register(&mut ctx);
    ctx
}

#[test]
fn registry_probes_and_demuxes_every_encodable_format() {
    let ctx = registered();
    let d = sample_design();
    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "dst",
            dst::encode(&d, &dst::DstEncodeOptions::default()).unwrap(),
        ),
        (
            "pec",
            pec::encode(&d, &pec::PecEncodeOptions::default()).unwrap(),
        ),
        (
            "hus",
            hus::encode(&d, &hus::HusEncodeOptions::default()).unwrap(),
        ),
        (
            "vip",
            hus::encode_vip(&d, &hus::HusEncodeOptions::default()).unwrap(),
        ),
        ("exp", oxideav_embroidery::exp::encode(&d).unwrap()),
    ];
    for (ext, bytes) in files {
        let mut cursor = Cursor::new(bytes.clone());
        let name = ctx
            .containers
            .probe_input(&mut cursor, Some(ext))
            .unwrap_or_else(|e| panic!("probe_input for {ext}: {e}"));
        assert_eq!(name, "embroidery");
        let demuxer = ctx
            .containers
            .open_demuxer(&name, Box::new(Cursor::new(bytes)), &NullCodecResolver)
            .unwrap();
        assert_eq!(demuxer.streams().len(), 1);
        let params = &demuxer.streams()[0].params;
        assert_eq!(params.codec_id.as_str(), "embroidery");
        assert_eq!(params.media_type, MediaType::Data);
        let meta = demuxer.metadata();
        let get = |k: &str| {
            meta.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("embroidery_format"), Some(ext));
        assert_eq!(get("stitches"), Some("3"));
        assert_eq!(get("color_blocks"), Some("2"));
    }
}

#[test]
fn demuxed_packet_decodes_to_a_vector_frame() {
    let ctx = registered();
    let bytes = dst::encode(&sample_design(), &dst::DstEncodeOptions::default()).unwrap();
    let mut demuxer = ctx
        .containers
        .open_demuxer(
            "embroidery",
            Box::new(Cursor::new(bytes)),
            &NullCodecResolver,
        )
        .unwrap();
    let packet = demuxer.next_packet().unwrap();
    assert!(matches!(demuxer.next_packet(), Err(CoreError::Eof)));

    let mut decoder = ctx
        .codecs
        .first_decoder(&demuxer.streams()[0].params)
        .unwrap();
    decoder.send_packet(&packet).unwrap();
    let frame = decoder.receive_frame().unwrap();
    let Frame::Vector(v) = frame else {
        panic!("expected a vector frame");
    };
    // Two colour blocks → two stroked paths; extents 0..30 on each
    // axis → a 31×31 canvas.
    assert_eq!(v.root.children.len(), 2);
    assert_eq!(v.width, 31.0);
    assert_eq!(v.height, 31.0);
    assert!(matches!(decoder.receive_frame(), Err(CoreError::NeedMore)));
    decoder.flush().unwrap();
    assert!(matches!(decoder.receive_frame(), Err(CoreError::Eof)));
}

#[test]
fn direct_factories_mirror_the_registry_path() {
    // The dual-API convention: make_demuxer / make_decoder without a
    // registry.
    let bytes = pec::encode(&sample_design(), &pec::PecEncodeOptions::default()).unwrap();
    let mut demuxer = oxideav_embroidery::make_demuxer(Box::new(Cursor::new(bytes))).unwrap();
    let packet = demuxer.next_packet().unwrap();
    let mut decoder = EmbroideryDecoder::new();
    decoder.send_packet(&packet).unwrap();
    assert!(matches!(decoder.receive_frame(), Ok(Frame::Vector(_))));
}

#[test]
fn probe_scores_follow_the_registry_convention() {
    let d = sample_design();
    let hus_bytes = hus::encode(&d, &hus::HusEncodeOptions::default()).unwrap();
    let dst_bytes = dst::encode(&d, &dst::DstEncodeOptions::default()).unwrap();
    let exp_bytes = oxideav_embroidery::exp::encode(&d).unwrap();
    let score = |buf: &[u8], ext: Option<&str>| framework::probe_container(&ProbeData { buf, ext });
    // Unambiguous magic → 100 regardless of extension.
    assert_eq!(score(&hus_bytes, None), MAX_PROBE_SCORE);
    // Structural probe: 75 with extension corroboration, 50 without.
    assert_eq!(score(&dst_bytes, Some("dst")), 75);
    assert_eq!(score(&dst_bytes, None), 50);
    // Headerless EXP: extension only.
    assert_eq!(score(&exp_bytes, Some("exp")), PROBE_SCORE_EXTENSION);
    assert_eq!(score(&exp_bytes, None), 0);
    assert_eq!(score(b"garbage", Some("txt")), 0);
}

#[test]
fn extension_map_covers_the_family() {
    let ctx = registered();
    for ext in framework::EXTENSIONS {
        assert_eq!(
            ctx.containers.container_for_extension(ext),
            Some("embroidery"),
            "extension {ext}"
        );
    }
    assert_eq!(ctx.containers.container_for_extension("vp3"), None);
    assert_eq!(ctx.containers.container_for_extension("edr"), None);
}

#[test]
fn vector_rendering_uses_thread_colors_and_flips_y() {
    let v = oxideav_embroidery::design_to_vector(&sample_design());
    use oxideav_core::vector::{Node, Paint};
    let strokes: Vec<[u8; 3]> = v
        .root
        .children
        .iter()
        .map(|n| {
            let Node::Path(p) = n else {
                panic!("path node")
            };
            let Some(stroke) = &p.stroke else {
                panic!("stroked")
            };
            let Paint::Solid(c) = &stroke.paint else {
                panic!("solid")
            };
            [c.r, c.g, c.b]
        })
        .collect();
    assert_eq!(strokes, vec![[255, 0, 0], [0, 0, 255]]);
    // The design's topmost needle position (model y = 30 = max_y)
    // must map to device y = 0.
    let Node::Path(p) = &v.root.children[0] else {
        panic!()
    };
    use oxideav_core::vector::PathCommand;
    let min_device_y = p
        .path
        .commands
        .iter()
        .filter_map(|c| match c {
            PathCommand::MoveTo(pt) | PathCommand::LineTo(pt) => Some(pt.y),
            _ => None,
        })
        .fold(f32::INFINITY, f32::min);
    assert_eq!(min_device_y, 0.0);
}

#[test]
fn undecodable_input_is_rejected_by_the_demuxer() {
    let r = oxideav_embroidery::make_demuxer(Box::new(Cursor::new(vec![0u8; 64])));
    assert!(matches!(r, Err(CoreError::InvalidData(_))));
}
