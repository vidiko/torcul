// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod support;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use support::*;
use torcul_media::{MUXER_WRITTEN_TAGS, probe, remux, snapshot, verify};
use torcul_policy::{TitleMeta, plan};

#[test]
fn muxer_written_tags_allowlist_entries_proven_on_fixture() {
    assert_eq!(MUXER_WRITTEN_TAGS.len(), 2);
    let tmp = tempfile::tempdir().unwrap();
    let piped = tmp.path().join("piped.mkv");
    pipe_matroska_without_duration_tags(
        &fixtures().dir.path().join("f07_default_dropped.mkv"),
        &piped,
    );
    let src_snap = snapshot(&piped).unwrap();
    assert!(
        !src_snap.tags.contains_key("ENCODER"),
        "source must start without ENCODER SimpleTag: {:?}",
        src_snap.tags
    );
    assert!(
        src_snap
            .streams
            .iter()
            .all(|s| !s.tags.keys().any(|k| k.eq_ignore_ascii_case("DURATION"))),
        "source streams must start without DURATION"
    );
    let p = plan(
        &probe(&piped).unwrap(),
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    )
    .unwrap();
    let staged = tmp.path().join("staged.mkv");
    remux(&piped, &p, &staged).unwrap();
    let dst_snap = snapshot(&staged).unwrap();
    assert!(
        dst_snap.tags.contains_key("ENCODER"),
        "Matroska muxer must write global ENCODER"
    );
    assert!(
        dst_snap
            .streams
            .iter()
            .all(|s| s.tags.contains_key("DURATION")),
        "Matroska muxer must write per-stream DURATION"
    );
    let verdict = verify(&piped, &p, &staged).unwrap();
    assert!(verdict.pass, "{:?}", verdict.failures);
}

fn pipe_matroska_without_duration_tags(src: &Path, dst: &Path) {
    use wait_timeout::ChildExt;
    let out_file = fs::File::create(dst).unwrap();
    let mut child = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-i"])
        .arg(src)
        .args([
            "-map",
            "0",
            "-map_metadata",
            "-1",
            "-metadata:s:a:0",
            "language=fre",
            "-disposition:a:0",
            "default",
            "-metadata:s:a:1",
            "language=eng",
            "-metadata:s:s:0",
            "language=eng",
            "-fflags",
            "+bitexact",
            "-c",
            "copy",
            "-f",
            "matroska",
            "pipe:1",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(out_file)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let status = child
        .wait_timeout(Duration::from_secs(60))
        .unwrap()
        .unwrap();
    assert!(status.success());
}

#[test]
fn verify_rejects_added_tag_in_staging() {
    let src = fixtures().dir.path().join("f07_default_dropped.mkv");
    let tmp = tempfile::tempdir().unwrap();
    let staged = tmp.path().join("staged.mkv");
    let p = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    )
    .unwrap();
    remux(&src, &p, &staged).unwrap();
    let extra = tmp.path().join("extra-tag.mkv");
    let out = execute(
        std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-y", "-loglevel", "error", "-i"])
            .arg(&staged)
            .args([
                "-map",
                "0",
                "-c",
                "copy",
                "-metadata",
                "unexpected_injected_tag=1",
            ])
            .arg(&extra),
        Duration::from_secs(60),
    );
    assert!(out.success, "{}", out.stderr);
    let verdict = verify(&src, &p, &extra).unwrap();
    assert!(!verdict.pass, "added global tag must fail verify");
    assert!(
        verdict
            .failures
            .iter()
            .any(|f| f.contains("added:UNEXPECTED_INJECTED_TAG")),
        "{:?}",
        verdict.failures
    );
}

#[test]
fn invalid_and_truncated_inputs() {
    let temp = tempfile::tempdir().unwrap();
    assert!(probe(&temp.path().join("missing.mkv")).is_err());
    let text = temp.path().join("text.mkv");
    fs::write(&text, b"not media").unwrap();
    assert!(probe(&text).is_err());
    let truncated = temp.path().join("truncated.mkv");
    let full = fs::read(fixtures().dir.path().join("f01_movie_en_de_extra.mkv")).unwrap();
    fs::write(&truncated, &full[..4096]).unwrap();
    assert!(probe(&truncated).is_err());
}

#[test]
fn tampered_packet_fails_lossless_proof() {
    let src = fixtures().dir.path().join("f08_mp4_en_fr.mp4");
    let tmp = tempfile::tempdir().unwrap();
    let dst = tmp.path().join("tampered.mp4");
    let p = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    )
    .unwrap();
    remux(&src, &p, &dst).unwrap();
    tamper_packet(&dst);
    let v = verify(&src, &p, &dst).unwrap();
    assert!(!v.pass);
    assert!(
        v.failures.iter().any(|s| s.contains("payload")),
        "{:?}",
        v.failures
    );
}

fn tamper_packet(path: &Path) {
    let end = Instant::now() + Duration::from_secs(30);
    let mut input =
        ffmpeg_next::format::input_with_interrupt(&path, move || Instant::now() >= end).unwrap();
    let mut payload = None;
    for _ in 0..1000 {
        let mut p = ffmpeg_next::Packet::empty();
        p.read(&mut input).unwrap();
        if p.data().is_some_and(|bytes| bytes.len() > 100) {
            payload = Some(p.data().unwrap().to_vec());
            break;
        }
    }
    let payload = payload.unwrap();
    let mut bytes = fs::read(path).unwrap();
    let offset = bytes
        .windows(payload.len())
        .position(|w| w == payload)
        .unwrap();
    bytes[offset + payload.len() / 2] ^= 1;
    fs::write(path, bytes).unwrap();
}

#[test]
fn wrong_plan_fails_verification() {
    let src = fixtures().dir.path().join("f02_anime_ja.mkv");
    let tmp = tempfile::tempdir().unwrap();
    let anime = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("ja".into()),
            anime: true,
        },
    )
    .unwrap();
    let movie = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("ja".into()),
            anime: false,
        },
    )
    .unwrap();
    let dst = tmp.path().join("movie.mkv");
    remux(&src, &movie, &dst).unwrap();
    assert!(!verify(&src, &anime, &dst).unwrap().pass);
}

#[test]
fn existing_staging_not_clobbered() {
    let src = fixtures().dir.path().join("f07_default_dropped.mkv");
    let tmp = tempfile::tempdir().unwrap();
    let dst = tmp.path().join("existing.mkv");
    fs::write(&dst, b"retain me").unwrap();
    let p = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    )
    .unwrap();
    assert!(remux(&src, &p, &dst).is_err());
    assert_eq!(fs::read(dst).unwrap(), b"retain me");
}

#[test]
fn invalid_plan_leaves_no_partial() {
    let src = fixtures().dir.path().join("f02_anime_ja.mkv");
    let facts = probe(&src).unwrap();
    let mut p = plan(
        &facts,
        &TitleMeta {
            original: Some("ja".into()),
            anime: true,
        },
    )
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let dst = temp.path().join("partial.mkv");
    p.decisions[0].index = 999;
    assert!(remux(&src, &p, &dst).is_err());
    assert!(!dst.exists());
}

#[test]
fn partial_cleanup_after_operation_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let partial = temp.path().join("partial.mkv");
    fs::write(&partial, b"partial").unwrap();
    let deadline = torcul_media::io::Deadline::with_timeout(Duration::ZERO).unwrap();
    assert!(deadline.check().is_err());
    assert!(
        torcul_media::io::Deadline::with_timeout(Duration::from_secs(60))
            .unwrap()
            .check()
            .is_ok()
    );
    assert!(torcul_media::io::Deadline::with_timeout(Duration::MAX).is_err());
    deadline.remove_partial(&partial).unwrap();
    assert!(!partial.exists());
}
