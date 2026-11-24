// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use super::*;
use crate::model::{ChannelLayout, PacketProof, SideDataKind, StreamCounts};
use std::collections::{BTreeMap, BTreeSet};
use torcul_policy::{TrackDecision, TrackInfo};

fn sample_packet() -> PacketRecord {
    PacketRecord {
        size: 512,
        pts: Some(100),
        dts: Some(90),
        duration: Some(40),
        key: true,
        side_data: Box::default(),
    }
}

fn sample_codec() -> CodecParams {
    CodecParams {
        codec_id: 27,
        profile: 100,
        level: 40,
        width: 1920,
        height: 1080,
        format: 0,
        frame_rate: TimeBase { num: 24, den: 1 },
        sample_rate: 0,
        channel_layout: ChannelLayout::default(),
        extradata: vec![1, 2, 3, 4],
        coded_side_data: Vec::new(),
    }
}

fn sample_stream(index: usize, kind: TrackKind, attached_pic: bool) -> StreamSnapshot {
    StreamSnapshot {
        track: TrackInfo {
            index,
            kind,
            codec: "h264".into(),
            language: Some("eng".into()),
            title: Some("Main".into()),
            dispositions: BTreeSet::new(),
            attached_pic,
        },
        time_base: TimeBase { num: 1, den: 1000 },
        codec_params: sample_codec(),
        tags: BTreeMap::from([("language".into(), "eng".into())]),
        proof: PacketProof {
            count: 1,
            bytes: 512,
            sha256: "deadbeef".into(),
        },
        packets: vec![sample_packet()],
    }
}

#[test]
fn packet_record_memory_size_is_bounded() {
    assert_eq!(std::mem::size_of::<PacketRecord>(), 80);
}

#[test]
fn synthetic_packet_size_key_and_nopts_checks() {
    let tb = TimeBase { num: 1, den: 1000 };
    let base = sample_packet();

    let mut diff_size = base.clone();
    diff_size.size = 513;
    let mut f = Vec::new();
    compare_packets(
        tb,
        std::slice::from_ref(&base),
        tb,
        &[diff_size],
        "s0",
        &mut f,
    );
    assert!(f.iter().any(|e| e.contains("size")), "{f:?}");

    let mut diff_key = base.clone();
    diff_key.key = false;
    let mut f = Vec::new();
    compare_packets(
        tb,
        std::slice::from_ref(&base),
        tb,
        &[diff_key],
        "s0",
        &mut f,
    );
    assert!(f.iter().any(|e| e.contains("key")), "{f:?}");

    let mut nopts = base.clone();
    nopts.pts = None;
    let mut f = Vec::new();
    compare_packets(
        tb,
        std::slice::from_ref(&base),
        tb,
        std::slice::from_ref(&nopts),
        "s0",
        &mut f,
    );
    assert!(f.iter().any(|e| e.contains("pts")), "{f:?}");

    let mut f = Vec::new();
    compare_packets(
        tb,
        std::slice::from_ref(&nopts),
        tb,
        &[nopts.clone()],
        "s0",
        &mut f,
    );
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn synthetic_packet_timestamp_coarse_tick_boundaries() {
    let tb_fine = TimeBase {
        num: 1,
        den: 48_000,
    };
    let tb_coarse = TimeBase { num: 1, den: 1000 };
    let base = sample_packet();

    let mut plus_one = base.clone();
    plus_one.pts = Some(101);
    let mut f = Vec::new();
    compare_packets(
        tb_coarse,
        std::slice::from_ref(&base),
        tb_coarse,
        &[plus_one],
        "s0",
        &mut f,
    );
    assert!(f.is_empty(), "1 coarse tick must pass: {f:?}");

    let fine_pkt = PacketRecord {
        pts: Some(4_800),
        dts: Some(4_320),
        duration: Some(1_920),
        ..base.clone()
    };
    let mut coarse_plus_one = base.clone();
    coarse_plus_one.pts = Some(101);
    let mut f = Vec::new();
    compare_packets(
        tb_fine,
        std::slice::from_ref(&fine_pkt),
        tb_coarse,
        &[coarse_plus_one],
        "s0",
        &mut f,
    );
    assert!(f.is_empty(), "rescaled 1 coarse tick must pass: {f:?}");

    let mut plus_two = base;
    plus_two.pts = Some(102);
    let mut f = Vec::new();
    compare_packets(tb_fine, &[fine_pkt], tb_coarse, &[plus_two], "s0", &mut f);
    assert!(f.iter().any(|e| e.contains("pts")), "{f:?}");
}

#[test]
fn synthetic_codec_params_and_side_data_checks() {
    let base = sample_codec();

    let mut bad_extra = base.clone();
    bad_extra.extradata = vec![1, 2, 3, 9];
    let mut f = Vec::new();
    compare_codec_params(&base, &bad_extra, false, "s0", &mut f);
    assert!(f.iter().any(|e| e.contains("extradata")), "{f:?}");

    let mut bad_profile = base.clone();
    bad_profile.profile = 77;
    let mut f = Vec::new();
    compare_codec_params(&base, &bad_profile, false, "s0", &mut f);
    assert!(f.iter().any(|e| e.contains("profile")), "{f:?}");

    let sd_a = SideDataRecord {
        kind: SideDataKind::MasteringDisplayMetadata,
        data: vec![10, 20, 30].into_boxed_slice(),
    };
    let sd_b = SideDataRecord {
        kind: SideDataKind::ContentLightLevel,
        data: vec![1, 2].into_boxed_slice(),
    };

    let mut f = Vec::new();
    compare_side_data(
        &[sd_a.clone(), sd_b],
        std::slice::from_ref(&sd_a),
        "s0-sd",
        &mut f,
    );
    assert!(f.iter().any(|e| e.contains("types")), "{f:?}");

    let sd_a_mut = SideDataRecord {
        kind: SideDataKind::MasteringDisplayMetadata,
        data: vec![10, 20, 99].into_boxed_slice(),
    };
    let mut f = Vec::new();
    compare_side_data(&[sd_a], &[sd_a_mut], "s0-sd", &mut f);
    assert!(f.iter().any(|e| e.contains("bytes")), "{f:?}");
}

#[test]
fn synthetic_tag_bidirectional_and_muxer_allowlist() {
    let src = Tags::from([
        ("language".into(), "eng".into()),
        ("title".into(), "Track".into()),
    ]);

    let mut added = src.clone();
    added.insert("UNEXPECTED".into(), "1".into());
    let mut f = Vec::new();
    compare_tags(&src, &added, "matroska:stream", "s0", &mut f);
    assert!(f.iter().any(|e| e.contains("added:UNEXPECTED")), "{f:?}");

    let mut removed = src.clone();
    removed.remove("title");
    let mut f = Vec::new();
    compare_tags(&src, &removed, "matroska:stream", "s0", &mut f);
    assert!(f.iter().any(|e| e.contains("missing:title")), "{f:?}");

    let mut with_duration = src.clone();
    with_duration.insert("DURATION".into(), "00:00:02.000000000".into());
    let mut f = Vec::new();
    compare_tags(&src, &with_duration, "matroska:stream", "s0", &mut f);
    assert!(f.is_empty(), "{f:?}");

    let global_dst = Tags::from([("encoder".into(), "Lavf63.16.100".into())]);
    let mut f = Vec::new();
    compare_tags(
        &Tags::new(),
        &global_dst,
        "matroska:global",
        "global",
        &mut f,
    );
    assert!(f.is_empty(), "{f:?}");

    let mut f = Vec::new();
    compare_tags(&src, &with_duration, "other:stream", "s0", &mut f);
    assert!(f.iter().any(|e| e.contains("added:DURATION")), "{f:?}");
}

#[test]
fn synthetic_counts_differ_and_equal() {
    let expected = MediaCounts {
        streams: StreamCounts {
            video: 1,
            audio: 3,
            subtitle: 3,
            attachment: 2,
        },
        chapters: 3,
        attachments: 2,
    };
    assert!(compare_expected_counts(&expected, &expected).is_empty());

    let mut bad_chapters = expected.clone();
    bad_chapters.chapters = 2;
    let f = compare_expected_counts(&expected, &bad_chapters);
    assert!(f.iter().any(|e| e.contains("chapter-count")), "{f:?}");

    let mut bad_attachments = expected.clone();
    bad_attachments.attachments = 1;
    let f = compare_expected_counts(&expected, &bad_attachments);
    assert!(f.iter().any(|e| e.contains("attachment-count")), "{f:?}");
}

#[test]
fn explicit_matroska_cover_mapping() {
    let src = MediaSnapshot {
        container: "matroska,webm".into(),
        streams: vec![
            sample_stream(0, TrackKind::Video, false),
            sample_stream(1, TrackKind::Video, true),
            sample_stream(2, TrackKind::Audio, false),
        ],
        chapters: Vec::new(),
        tags: Tags::new(),
    };
    let plan = Plan {
        decisions: (0..3)
            .map(|index| TrackDecision {
                index,
                keep: true,
                reason: "keep".into(),
                lid: None,
            })
            .collect(),
        warnings: Vec::new(),
        default_audio_change: None,
        needs_remux: false,
    };
    assert_eq!(
        build_stream_map(&src, &plan),
        vec![
            StreamMapping {
                source: 0,
                staging: 0,
                attached_pic: false,
            },
            StreamMapping {
                source: 2,
                staging: 1,
                attached_pic: false,
            },
            StreamMapping {
                source: 1,
                staging: 2,
                attached_pic: true,
            },
        ]
    );
}
