// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod support;
use std::{
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};
use support::*;
use torcul_media::{DURATION_TOLERANCE_US, duration_within_tolerance, probe, remux, verify};
use torcul_policy::{TitleMeta, plan};

#[test]
fn oracle_all_eleven() {
    let fx = fixtures();
    assert_eq!(fx.manifest.fixtures.len(), 11);
    for row in &fx.manifest.fixtures {
        let facts = probe(&fx.dir.path().join(&row.name)).unwrap();
        let p = plan(
            &facts,
            &TitleMeta {
                original: row.original.clone(),
                anime: row.anime,
            },
        );
        assert_eq!(p.kept_indices(), row.expect, "{}", row.name);
        assert_eq!(p.warnings, row.warnings, "{}", row.name);
        assert_eq!(p.needs_remux, row.needs_remux, "{}", row.name);
    }
}
#[test]
fn remux_and_verify_every_fixture() {
    let fx = fixtures();
    let staging = tempfile::tempdir().unwrap();
    for row in &fx.manifest.fixtures {
        let src = fx.dir.path().join(&row.name);
        let dst = staging.path().join(&row.name);
        let p = plan(
            &probe(&src).unwrap(),
            &TitleMeta {
                original: row.original.clone(),
                anime: row.anime,
            },
        );
        remux(&src, &p, &dst).unwrap_or_else(|e| panic!("{}: {e}", row.name));
        if row.needs_remux {
            let verdict = verify(&src, &p, &dst).unwrap();
            assert!(verdict.pass, "{}: {:?}", row.name, verdict.failures);
        } else {
            assert!(!dst.exists(), "{} must write nothing", row.name);
        }
    }
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
    );
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
    );
    let movie = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("ja".into()),
            anime: false,
        },
    );
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
    );
    assert!(remux(&src, &p, &dst).is_err());
    assert_eq!(fs::read(dst).unwrap(), b"retain me");
}
#[test]
fn duration_tolerance_boundary() {
    assert!(duration_within_tolerance(Some(1_000_000), Some(1_000_000)));
    assert!(duration_within_tolerance(
        Some(0),
        Some(DURATION_TOLERANCE_US as i64)
    ));
    assert!(!duration_within_tolerance(
        Some(0),
        Some(DURATION_TOLERANCE_US as i64 + 1)
    ));
    assert!(!duration_within_tolerance(Some(0), None));
    assert!(duration_within_tolerance(None, None));
}
#[test]
fn cli_dry_run_report_matches_oracle() {
    let fx = fixtures();
    let tmp = tempfile::tempdir().unwrap();
    let json = tmp.path().join("plan.json");
    let meta = fx.dir.path().join("manifest.json");
    let out = cli(&[
        "plan".as_ref(),
        "--meta".as_ref(),
        meta.as_os_str(),
        "--json".as_ref(),
        json.as_os_str(),
        fx.dir.path().as_os_str(),
    ]);
    assert!(out.success, "{}", out.stderr);
    println!("{}", out.stdout);
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_eq!(rows.len(), 11);
    for (report, oracle) in rows.iter().zip(&fx.manifest.fixtures) {
        let p: torcul_policy::Plan = serde_json::from_value(report["plan"].clone()).unwrap();
        assert_eq!(p.kept_indices(), oracle.expect);
        assert_eq!(p.warnings, oracle.warnings);
        assert_eq!(p.needs_remux, oracle.needs_remux);
    }
}
#[test]
fn cli_missing_metadata_and_empty_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta.json");
    fs::write(&meta, br#"{"fixtures":[]}"#).unwrap();
    let empty = tempfile::tempdir().unwrap();
    let out = command("plan", Some(&meta), None, empty.path());
    assert!(out.success, "{}", out.stderr);
    let file = fixtures().dir.path().join("f01_movie_en_de_extra.mkv");
    let out = command("plan", Some(&meta), None, &file);
    assert!(!out.success);
    assert!(out.stdout.contains("no metadata entry"));
}
fn prepared(name: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let (temp, copy) = copy_fixture(name);
    let staging = temp.path().join("staging");
    let meta = fixtures().dir.path().join("manifest.json");
    let out = command("remux", Some(&meta), Some(&staging), &copy);
    assert!(out.success, "{}", out.stderr);
    (temp, copy, staging)
}
#[test]
fn cli_apply_requires_verification_then_replaces_copy() {
    let (_tmp, src, staging) = prepared("f07_default_dropped.mkv");
    let before = fs::read(&src).unwrap();
    let out = command("apply", None, Some(&staging), &src);
    assert!(!out.success);
    assert_eq!(fs::read(&src).unwrap(), before);
    let out = command("verify", None, Some(&staging), &src);
    assert!(out.success, "{}", out.stderr);
    let out = command("apply", None, Some(&staging), &src);
    assert!(out.success, "{}", out.stderr);
    assert_eq!(probe(&src).unwrap().tracks.len(), 3);
    assert!(
        probe(&src).unwrap().tracks[1]
            .dispositions
            .contains("default")
    );
}
#[test]
fn cli_apply_refuses_changed_source() {
    let (_tmp, src, staging) = prepared("f07_default_dropped.mkv");
    let out = command("verify", None, Some(&staging), &src);
    assert!(out.success, "{}", out.stderr);
    fs::OpenOptions::new()
        .append(true)
        .open(&src)
        .unwrap()
        .write_all(b"x")
        .unwrap();
    let before = fs::read(&src).unwrap();
    let out = command("apply", None, Some(&staging), &src);
    assert!(!out.success);
    assert_eq!(fs::read(src).unwrap(), before);
}
#[test]
fn cli_apply_refuses_changed_staging() {
    let (_tmp, src, staging) = prepared("f08_mp4_en_fr.mp4");
    let out = command("verify", None, Some(&staging), &src);
    assert!(out.success, "{}", out.stderr);
    tamper_packet(&staging.join(src.file_name().unwrap()));
    let out = command("apply", None, Some(&staging), &src);
    assert!(!out.success);
    let out = command("verify", None, Some(&staging), &src);
    assert!(!out.success);
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(staging.join("f08_mp4_en_fr.mp4.torcul.json")).unwrap())
            .unwrap();
    assert_eq!(record["verification"]["pass"], false);
}
#[test]
fn cli_clean_files_write_nothing() {
    for name in ["f06_already_clean.mkv", "f11_no_audio_matches_original.mkv"] {
        let (temp, src) = copy_fixture(name);
        let staging = temp.path().join("staging");
        let out = command(
            "remux",
            Some(&fixtures().dir.path().join("manifest.json")),
            Some(&staging),
            &src,
        );
        assert!(out.success, "{}", out.stderr);
        assert!(!staging.exists());
    }
}

#[test]
fn chapters_and_global_metadata_round_trip() {
    for name in ["f07_default_dropped.mkv", "f08_mp4_en_fr.mp4"] {
        let temp = tempfile::tempdir().unwrap();
        let src = enriched(temp.path(), name, 900, "retain");
        let facts = probe(&src).unwrap();
        let p = plan(
            &facts,
            &TitleMeta {
                original: Some("en".into()),
                anime: false,
            },
        );
        let dst = temp.path().join(format!("output-{}", name));
        remux(&src, &p, &dst).unwrap();
        let verdict = verify(&src, &p, &dst).unwrap();
        assert!(verdict.pass, "{name}: {:?}", verdict.failures);
        let end = Instant::now() + Duration::from_secs(30);
        let input =
            ffmpeg_next::format::input_with_interrupt(&dst, move || Instant::now() >= end).unwrap();
        assert_eq!(input.nb_chapters(), 1);
        assert!(
            input
                .metadata()
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("torcul_note") && v == "retain")
        );
    }
}
fn enriched(dir: &Path, name: &str, chapter_end: u32, note: &str) -> std::path::PathBuf {
    let meta = dir.join(format!("{note}.ffmeta"));
    fs::write(
        &meta,
        format!(
            ";FFMETADATA1
title=Pilot
torcul_note={note}
[CHAPTER]
TIMEBASE=1/1000
START=0
END={chapter_end}
title=Opening
"
        ),
    )
    .unwrap();
    let dst = dir.join(format!("{note}-{name}"));
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-nostdin", "-y", "-loglevel", "error", "-i"])
        .arg(fixtures().dir.path().join(name));
    cmd.arg("-i").arg(meta).args([
        "-map",
        "0",
        "-map_metadata",
        "1",
        "-map_chapters",
        "1",
        "-c",
        "copy",
    ]);
    if name.ends_with(".mp4") {
        cmd.args(["-movflags", "use_metadata_tags"]);
    }
    let out = execute(cmd.arg(&dst), Duration::from_secs(60));
    assert!(out.success, "{}", out.stderr);
    dst
}
#[test]
fn changed_chapters_and_metadata_fail_verification() {
    let temp = tempfile::tempdir().unwrap();
    let src = enriched(temp.path(), "f07_default_dropped.mkv", 900, "retain");
    let other = enriched(temp.path(), "f07_default_dropped.mkv", 800, "changed");
    let p = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    );
    let dst = temp.path().join("changed.mkv");
    remux(&other, &p, &dst).unwrap();
    let verdict = verify(&src, &p, &dst).unwrap();
    assert!(!verdict.pass);
    assert!(
        verdict.failures.iter().any(|s| s == "chapters"),
        "{:?}",
        verdict.failures
    );
    assert!(
        verdict
            .failures
            .iter()
            .any(|s| s.contains("global-metadata")),
        "{:?}",
        verdict.failures
    );
}
#[test]
fn verify_changed_source_records_failure() {
    let (_tmp, src, staging) = prepared("f07_default_dropped.mkv");
    let out = command("verify", None, Some(&staging), &src);
    assert!(out.success, "{}", out.stderr);
    fs::OpenOptions::new()
        .append(true)
        .open(&src)
        .unwrap()
        .write_all(b"x")
        .unwrap();
    let out = command("verify", None, Some(&staging), &src);
    assert!(!out.success);
    let record: serde_json::Value = serde_json::from_slice(
        &fs::read(staging.join("f07_default_dropped.mkv.torcul.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["verification"]["pass"], false);
}

#[test]
fn unknown_language_changes_are_detected() {
    let temp = tempfile::tempdir().unwrap();
    let src = retagged(temp.path(), "qzz");
    let changed = retagged(temp.path(), "qzy");
    let p = plan(&probe(&src).unwrap(), &TitleMeta::default());
    let good = temp.path().join("good.mkv");
    remux(&src, &p, &good).unwrap();
    assert!(verify(&src, &p, &good).unwrap().pass);
    let bad = temp.path().join("bad.mkv");
    remux(&changed, &p, &bad).unwrap();
    assert!(
        !verify(&src, &p, &bad).unwrap().pass,
        "different unknown languages must not collapse to None"
    );
}
fn retagged(dir: &Path, lang: &str) -> std::path::PathBuf {
    let dst = dir.join(format!("{lang}.mkv"));
    let out = execute(
        std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-y", "-loglevel", "error", "-i"])
            .arg(fixtures().dir.path().join("f03_foreign_only_fr.mkv"))
            .args([
                "-map",
                "0",
                "-c",
                "copy",
                "-metadata:s:a:0",
                &format!("language={lang}"),
            ])
            .arg(&dst),
        Duration::from_secs(60),
    );
    assert!(out.success, "{}", out.stderr);
    dst
}
#[test]
fn mp4_attached_picture_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("cover.mp4");
    let out = execute(
        std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-y", "-loglevel", "error", "-i"])
            .arg(fixtures().dir.path().join("f08_mp4_en_fr.mp4"))
            .arg("-i")
            .arg(fixtures().dir.path().join("f01_movie_en_de_extra.mkv"))
            .args([
                "-map",
                "0",
                "-map",
                "1:v:1",
                "-c",
                "copy",
                "-disposition:v:1",
                "attached_pic",
            ])
            .arg(&src),
        Duration::from_secs(60),
    );
    assert!(out.success, "{}", out.stderr);
    let facts = probe(&src).unwrap();
    assert!(facts.tracks.iter().any(|t| t.attached_pic));
    let p = plan(
        &facts,
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    );
    let dst = temp.path().join("output.mp4");
    remux(&src, &p, &dst).unwrap();
    let verdict = verify(&src, &p, &dst).unwrap();
    assert!(verdict.pass, "{:?}", verdict.failures);
}
#[test]
fn fingerprint_windows_and_full_digest_boundary() {
    for size in [0, 1, 1024 * 1024, 3 * 1024 * 1024] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bytes");
        let bytes = vec![42; size];
        fs::write(&path, &bytes).unwrap();
        let before = torcul_media::io::fingerprint(&path).unwrap();
        assert_eq!(before.size, size as u64);
        assert_eq!(before, torcul_media::io::fingerprint(&path).unwrap());
        let digest = torcul_media::io::file_sha256(&path).unwrap();
        if size > 0 {
            let mut changed = bytes;
            changed[size / 2] ^= 1;
            fs::write(&path, &changed).unwrap();
            assert_ne!(digest, torcul_media::io::file_sha256(&path).unwrap());
        }
    }
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
    );
    let temp = tempfile::tempdir().unwrap();
    let dst = temp.path().join("partial.mkv");
    p.decisions[0].index = 999;
    assert!(remux(&src, &p, &dst).is_err());
    assert!(!dst.exists());
}

#[test]
fn original_encoder_metadata_is_preserved() {
    for name in ["f07_default_dropped.mkv", "f08_mp4_en_fr.mp4"] {
        let (_temp, src) = copy_fixture(name);
        let end = Instant::now() + Duration::from_secs(30);
        let input =
            ffmpeg_next::format::input_with_interrupt(&src, move || Instant::now() >= end).unwrap();
        let encoder = input
            .metadata()
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("encoder"))
            .unwrap()
            .1
            .as_bytes()
            .to_vec();
        drop(input);
        let mut bytes = fs::read(&src).unwrap();
        let replacement = vec![b'x'; encoder.len()];
        for i in 0..=bytes.len() - encoder.len() {
            if bytes[i..i + encoder.len()] == encoder {
                bytes[i..i + encoder.len()].copy_from_slice(&replacement);
            }
        }
        fs::write(&src, bytes).unwrap();
        let p = plan(
            &probe(&src).unwrap(),
            &TitleMeta {
                original: Some("en".into()),
                anime: false,
            },
        );
        let dst = src.with_file_name(format!("out-{name}"));
        remux(&src, &p, &dst).unwrap();
        let end = Instant::now() + Duration::from_secs(30);
        let output =
            ffmpeg_next::format::input_with_interrupt(&dst, move || Instant::now() >= end).unwrap();
        assert_eq!(
            output
                .metadata()
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("encoder"))
                .unwrap()
                .1
                .as_bytes(),
            replacement
        );
        assert!(verify(&src, &p, &dst).unwrap().pass);
    }
}

#[test]
fn mp4_changed_file_type_size_rescales_chunk_offsets() {
    let temp = tempfile::tempdir().unwrap();
    let src = fragmented(temp.path(), false);
    let original = fixtures().dir.path().join("f08_mp4_en_fr.mp4");
    assert_ne!(
        ftyp_size(&src),
        ftyp_size(&original),
        "fixture must exercise nonzero header-size delta"
    );
    let p = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    );
    let dst = temp.path().join("output.mp4");
    remux(&src, &p, &dst).unwrap();
    let verdict = verify(&src, &p, &dst).unwrap();
    assert!(verdict.pass, "{:?}", verdict.failures);
    assert_eq!(ftyp_size(&src), ftyp_size(&dst));
}
fn fragmented(dir: &Path, subtitles: bool) -> std::path::PathBuf {
    let src = dir.join("fragmented.mp4");
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-nostdin", "-y", "-loglevel", "error", "-i"])
        .arg(fixtures().dir.path().join("f08_mp4_en_fr.mp4"));
    if subtitles {
        cmd.args(["-map", "0"]);
    } else {
        cmd.args(["-map", "0:v", "-map", "0:a"]);
    }
    let out = execute(
        cmd.args([
            "-c",
            "copy",
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof",
        ])
        .arg(&src),
        Duration::from_secs(60),
    );
    assert!(out.success, "{}", out.stderr);
    src
}
#[test]
fn fragmented_subtitles_require_exact_packet_proof() {
    let temp = tempfile::tempdir().unwrap();
    let src = fragmented(temp.path(), true);
    let meta = temp.path().join("meta.json");
    fs::write(
        &meta,
        br#"{"fixtures":[{"name":"fragmented.mp4","original":"en","anime":false}]}"#,
    )
    .unwrap();
    let staging = temp.path().join("staging");
    let out = command("remux", Some(&meta), Some(&staging), &src);
    assert!(out.success, "{}", out.stderr);
    let p = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    );
    let report = verify(&src, &p, &staging.join("fragmented.mp4")).unwrap();
    let exact = report.staging_packets.len() == p.kept_indices().len()
        && p.kept_indices()
            .iter()
            .enumerate()
            .all(|(to, from)| report.source_packets[*from] == report.staging_packets[to]);
    assert_eq!(report.pass, exact, "{:?}", report.failures);
    let out = command("verify", None, Some(&staging), &src);
    assert_eq!(out.success, report.pass);
    if !report.pass {
        let before = fs::read(&src).unwrap();
        let out = command("apply", None, Some(&staging), &src);
        assert!(!out.success);
        assert_eq!(fs::read(&src).unwrap(), before);
    }
}
fn ftyp_size(path: &Path) -> usize {
    use mp4_atom::ReadFrom;
    let bytes = fs::read(path).unwrap();
    let mut cursor = std::io::Cursor::new(bytes);
    let header = mp4_atom::Header::read_from(&mut cursor).unwrap();
    assert_eq!(header.kind, mp4_atom::FourCC::new(b"ftyp"));
    header.size.unwrap() + cursor.position() as usize
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
