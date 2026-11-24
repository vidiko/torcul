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
    pipe_matroska_without_duration_tags(&fixtures().dir.join("f07_default_dropped.mkv"), &piped);
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
    let out = execute_to(
        std::process::Command::new("ffmpeg")
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
            ]),
        dst,
        Duration::from_secs(60),
    );
    assert!(out.success, "{}", out.stderr);
}

#[test]
fn verify_rejects_added_tag_in_staging() {
    let src = fixtures().dir.join("f07_default_dropped.mkv");
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
    let full = fs::read(fixtures().dir.join("f01_movie_en_de_extra.mkv")).unwrap();
    fs::write(&truncated, &full[..4096]).unwrap();
    assert!(probe(&truncated).is_err());
}

#[test]
fn tampered_packet_fails_lossless_proof() {
    let src = fixtures().dir.join("f08_mp4_en_fr.mp4");
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
    let src = fixtures().dir.join("f02_anime_ja.mkv");
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
    let src = fixtures().dir.join("f07_default_dropped.mkv");
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
    let src = fixtures().dir.join("f02_anime_ja.mkv");
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

#[test]
fn default_audio_change_rejects_invalid_targets() {
    let src = fixtures().dir.join("f07_default_dropped.mkv");
    let facts = probe(&src).unwrap();
    let base = plan(
        &facts,
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    )
    .unwrap();
    let (dropped_from, _) = base.default_audio_change.unwrap();
    let video_idx = facts
        .tracks
        .iter()
        .find(|t| t.kind == torcul_policy::TrackKind::Video)
        .unwrap()
        .index;

    let temp = tempfile::tempdir().unwrap();
    let valid_dst = temp.path().join("valid.mkv");
    remux(&src, &base, &valid_dst).unwrap();
    assert!(verify(&src, &base, &valid_dst).unwrap().pass);

    for (label, bad_to) in [
        ("dropped", dropped_from),
        ("video", video_idx),
        ("boundary-oob", facts.tracks.len()),
        ("max-oob", usize::MAX),
    ] {
        let mut bad = base.clone();
        bad.default_audio_change = Some((dropped_from, bad_to));
        bad.needs_remux = true;
        let dst = temp.path().join(format!("{label}.mkv"));
        assert!(
            remux(&src, &bad, &dst).is_err(),
            "{label} target {bad_to} must be rejected"
        );
        assert!(!dst.exists(), "{label} must leave no staging file");
    }
}

#[test]
fn fixtures_dir_lives_under_cargo_target_tmpdir() {
    let expected = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("{}-fixtures", env!("CARGO_CRATE_NAME")));
    assert_eq!(&*fixtures().dir, expected.as_path());

    let temp = tempfile::tempdir().unwrap();
    let scratch = temp.path().join("reset-check");
    fs::create_dir_all(&scratch).unwrap();
    let stale = scratch.join("stale.txt");
    fs::write(&stale, b"leftover").unwrap();
    reset_fixture_dir(&scratch);
    assert!(scratch.is_dir());
    assert!(!stale.exists());
}

#[test]
fn fixtures_holds_exclusive_cross_process_lock() {
    let _fx = fixtures();
    let lock_path = fixture_lock_path();
    let second = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    assert!(
        matches!(second.try_lock(), Err(fs::TryLockError::WouldBlock)),
        "expected active fixture lock on {}",
        lock_path.display()
    );
}

#[test]
fn execute_to_streams_stdout_drains_large_stderr_and_kills_on_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let out_path = temp.path().join("stdout.bin");
    let ok = execute_to(
        std::process::Command::new("python3").args([
            "-I",
            "-c",
            "import sys; sys.stdout.buffer.write(bytes([0, 255, 111, 107])); sys.stderr.write('e' * 131072)",
        ]),
        &out_path,
        Duration::from_secs(10),
    );
    assert!(ok.success);
    assert_eq!(fs::read(&out_path).unwrap(), b"\x00\xffok");
    assert!(ok.stderr.ends_with(&"e".repeat(131_072)));

    let fail_path = temp.path().join("fail.bin");
    let fail = execute_to(
        std::process::Command::new("python3").args([
            "-I",
            "-c",
            "import sys; sys.stderr.write('boom'); sys.exit(2)",
        ]),
        &fail_path,
        Duration::from_secs(10),
    );
    assert!(!fail.success);
    assert!(fail.stderr.ends_with("boom"));

    let timeout_path = temp.path().join("timeout.bin");
    let started = Instant::now();
    let panicked = std::panic::catch_unwind(|| {
        execute_to(
            std::process::Command::new("python3").args(["-I", "-c", "import time; time.sleep(10)"]),
            &timeout_path,
            Duration::from_millis(50),
        );
    });
    assert!(panicked.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}
