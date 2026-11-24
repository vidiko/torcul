// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod support;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use support::*;
use torcul_media::{
    MUXER_WRITTEN_TAGS, StreamCounts, StreamMapping, expected_counts, probe, remux, snapshot,
    verify,
};
use torcul_policy::{FileFacts, Plan, TitleMeta, TrackKind, plan};

fn facts_stream_counts(facts: &FileFacts) -> StreamCounts {
    let mut counts = StreamCounts::default();
    for t in &facts.tracks {
        counts.add(t.kind);
    }
    counts
}

fn check_remuxed_fixture(
    src: &Path,
    dst: &Path,
    row: &Row,
    planned: &Plan,
) -> std::result::Result<(), String> {
    let src_facts = probe(src).map_err(|e| format!("probe source: {e}"))?;
    if facts_stream_counts(&src_facts) != row.counts.source {
        return Err(format!(
            "source-count-drift: probe {:?} != manifest {:?}",
            facts_stream_counts(&src_facts),
            row.counts.source
        ));
    }
    if src_facts.chapters != row.chapters {
        return Err(format!(
            "source-chapter-drift: probe {} != manifest {}",
            src_facts.chapters, row.chapters
        ));
    }
    let dst_facts = probe(dst).map_err(|e| format!("probe output: {e}"))?;
    let src_snap = snapshot(src).map_err(|e| format!("snapshot source: {e}"))?;
    let dst_snap = snapshot(dst).map_err(|e| format!("snapshot output: {e}"))?;
    let kept_expected = expected_counts(&src_snap, planned);
    let dst_stream_counts = facts_stream_counts(&dst_facts);
    if dst_stream_counts != row.counts.output || dst_stream_counts != kept_expected.streams {
        return Err(format!(
            "output-stream-counts: probe {dst_stream_counts:?}, manifest {:?}, plan {:?}",
            row.counts.output, kept_expected.streams
        ));
    }
    check_chapters_and_attachments(&src_snap, &dst_snap)?;
    let report = verify(src, planned, dst).map_err(|e| format!("verify: {e}"))?;
    if !report.pass {
        return Err(format!("verify-failed: {:?}", report.failures));
    }
    Ok(())
}

fn check_chapters_and_attachments(
    src_snap: &torcul_media::MediaSnapshot,
    dst_snap: &torcul_media::MediaSnapshot,
) -> std::result::Result<(), String> {
    if dst_snap.chapters.len() != src_snap.chapters.len() {
        return Err(format!(
            "chapter-count: expected {}, actual {}",
            src_snap.chapters.len(),
            dst_snap.chapters.len()
        ));
    }
    for (idx, (sc, dc)) in src_snap.chapters.iter().zip(&dst_snap.chapters).enumerate() {
        let start_eq = i128::from(sc.start)
            * i128::from(sc.time_base.num)
            * i128::from(dc.time_base.den)
            == i128::from(dc.start) * i128::from(dc.time_base.num) * i128::from(sc.time_base.den);
        let end_eq =
            i128::from(sc.end) * i128::from(sc.time_base.num) * i128::from(dc.time_base.den)
                == i128::from(dc.end) * i128::from(dc.time_base.num) * i128::from(sc.time_base.den);
        if !start_eq || !end_eq || sc.tags.get("title") != dc.tags.get("title") {
            return Err(format!("chapter-{idx}-mismatch"));
        }
    }
    check_attachments(src_snap, dst_snap)
}

fn check_attachments(
    src_snap: &torcul_media::MediaSnapshot,
    dst_snap: &torcul_media::MediaSnapshot,
) -> std::result::Result<(), String> {
    let src_att: Vec<_> = src_snap
        .streams
        .iter()
        .filter(|s| s.track.kind == TrackKind::Attachment)
        .collect();
    let dst_att: Vec<_> = dst_snap
        .streams
        .iter()
        .filter(|s| s.track.kind == TrackKind::Attachment)
        .collect();
    if dst_att.len() != src_att.len() {
        return Err(format!(
            "attachment-count: expected {}, actual {}",
            src_att.len(),
            dst_att.len()
        ));
    }
    for (idx, (sa, da)) in src_att.iter().zip(&dst_att).enumerate() {
        if sa.tags.get("filename") != da.tags.get("filename")
            || sa.tags.get("mimetype") != da.tags.get("mimetype")
            || sa.codec_params.extradata != da.codec_params.extradata
            || da.codec_params.extradata.is_empty()
        {
            return Err(format!("attachment-{idx}-mismatch"));
        }
    }
    Ok(())
}

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
        )
        .unwrap();
        assert_eq!(p.kept_indices(), row.expect, "{}", row.name);
        assert_eq!(p.warnings, row.warnings, "{}", row.name);
        assert_eq!(p.needs_remux, row.needs_remux, "{}", row.name);
    }
}

#[test]
fn remux_and_verify_every_fixture_with_counts_and_stream_map() {
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
        )
        .unwrap();
        remux(&src, &p, &dst).unwrap_or_else(|e| panic!("{}: {e}", row.name));
        if row.needs_remux {
            check_remuxed_fixture(&src, &dst, row, &p)
                .unwrap_or_else(|e| panic!("{}: {e}", row.name));
            if row.name == "f01_movie_en_de_extra.mkv" {
                let report = verify(&src, &p, &dst).unwrap();
                assert_eq!(
                    report.stream_map.last().copied(),
                    Some(StreamMapping {
                        source: 12,
                        staging: 8,
                        attached_pic: true,
                    })
                );
            }
        } else {
            assert!(!dst.exists(), "{} must write nothing", row.name);
        }
    }
}

#[test]
fn f02_chapters_attachments_and_counts_match_oracle() {
    let fx = fixtures();
    let row = fx
        .manifest
        .fixtures
        .iter()
        .find(|r| r.name == "f02_anime_ja.mkv")
        .unwrap();
    assert_eq!(row.chapters, 3);
    let src = fx.dir.path().join(&row.name);
    let tmp = tempfile::tempdir().unwrap();
    let dst = tmp.path().join(&row.name);
    let p = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: row.original.clone(),
            anime: row.anime,
        },
    )
    .unwrap();
    remux(&src, &p, &dst).unwrap();
    check_remuxed_fixture(&src, &dst, row, &p).unwrap();
    let report = verify(&src, &p, &dst).unwrap();
    println!(
        "f02 source_counts={:?} output_counts={:?}",
        report.source_counts, report.output_counts
    );
    assert_eq!(report.source_counts.chapters, 3);
    assert_eq!(report.output_counts.chapters, 3);
    assert_eq!(report.source_counts.attachments, 2);
    assert_eq!(report.output_counts.attachments, 2);
}

#[test]
fn f02_negative_mutations_fail_chapter_attachment_and_drift_asserts() {
    let fx = fixtures();
    let row = fx
        .manifest
        .fixtures
        .iter()
        .find(|r| r.name == "f02_anime_ja.mkv")
        .unwrap();
    let src = fx.dir.path().join(&row.name);
    let tmp = tempfile::tempdir().unwrap();
    let dst = tmp.path().join(&row.name);
    let p = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: row.original.clone(),
            anime: row.anime,
        },
    )
    .unwrap();
    remux(&src, &p, &dst).unwrap();

    let missing_chap = drop_one_chapter(tmp.path(), &dst);
    let err = check_remuxed_fixture(&src, &missing_chap, row, &p).unwrap_err();
    assert!(err.contains("chapter-count"), "{err}");
    let v = verify(&src, &p, &missing_chap).unwrap();
    assert!(!v.pass && v.failures.iter().any(|f| f.contains("chapter-count")));

    let missing_att = drop_one_attachment(tmp.path(), &dst);
    let err = check_remuxed_fixture(&src, &missing_att, row, &p).unwrap_err();
    assert!(
        err.contains("attachment-count") || err.contains("output-stream-counts"),
        "{err}"
    );
    let v = verify(&src, &p, &missing_att).unwrap();
    assert!(!v.pass && v.failures.iter().any(|f| f.contains("attachment-count")));

    let mut drifted = row.clone();
    drifted.counts.source.audio += 1;
    let err = check_remuxed_fixture(&src, &dst, &drifted, &p).unwrap_err();
    assert!(err.contains("source-count-drift"), "{err}");
}

fn drop_one_chapter(dir: &Path, dst: &Path) -> std::path::PathBuf {
    let two_chap_meta = dir.join("two.ffmeta");
    fs::write(
        &two_chap_meta,
        ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=666\ntitle=Chapter 1\n\
         [CHAPTER]\nTIMEBASE=1/1000\nSTART=666\nEND=1333\ntitle=Chapter 2\n",
    )
    .unwrap();
    let missing_chap = dir.join("missing-chapter.mkv");
    let out = execute(
        std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-y", "-loglevel", "error", "-i"])
            .arg(dst)
            .args(["-f", "ffmetadata", "-i"])
            .arg(&two_chap_meta)
            .args(["-map", "0", "-map_chapters", "1", "-c", "copy"])
            .arg(&missing_chap),
        Duration::from_secs(60),
    );
    assert!(out.success, "{}", out.stderr);
    missing_chap
}

fn drop_one_attachment(dir: &Path, dst: &Path) -> std::path::PathBuf {
    let missing_att = dir.join("missing-attachment.mkv");
    let out = execute(
        std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-y", "-loglevel", "error", "-i"])
            .arg(dst)
            .args(["-map", "0", "-map", "-0:t:1", "-c", "copy"])
            .arg(&missing_att),
        Duration::from_secs(60),
    );
    assert!(out.success, "{}", out.stderr);
    missing_att
}

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
        )
        .unwrap();
        let dst = temp.path().join(format!("output-{name}"));
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
            ";FFMETADATA1\ntitle=Pilot\ntorcul_note={note}\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND={chapter_end}\ntitle=Opening\n"
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
    )
    .unwrap();
    let dst = temp.path().join("changed.mkv");
    remux(&other, &p, &dst).unwrap();
    let verdict = verify(&src, &p, &dst).unwrap();
    assert!(!verdict.pass);
    assert!(
        verdict.failures.iter().any(|s| s.starts_with("chapters:")),
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
fn unknown_language_changes_are_detected() {
    let temp = tempfile::tempdir().unwrap();
    let src = retagged(temp.path(), "qzz");
    let changed = retagged(temp.path(), "qzy");
    let p = plan(&probe(&src).unwrap(), &TitleMeta::default()).unwrap();
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
    )
    .unwrap();
    let dst = temp.path().join("output.mp4");
    remux(&src, &p, &dst).unwrap();
    let verdict = verify(&src, &p, &dst).unwrap();
    assert!(verdict.pass, "{:?}", verdict.failures);
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
        )
        .unwrap();
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
    )
    .unwrap();
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
    let p = plan(
        &probe(&src).unwrap(),
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    )
    .unwrap();
    let staged = temp.path().join("staged-fragmented.mp4");
    remux(&src, &p, &staged).unwrap();
    let report = verify(&src, &p, &staged).unwrap();
    let exact = report.staging_packets.len() == p.kept_indices().len()
        && report
            .stream_map
            .iter()
            .all(|m| report.source_packets[m.source] == report.staging_packets[m.staging]);
    if !exact {
        assert!(!report.pass, "{:?}", report.failures);
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
