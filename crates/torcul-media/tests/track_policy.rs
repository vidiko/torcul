// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod support;
use std::{fs, path::Path, time::Duration};
use support::*;
use torcul_media::{StreamCounts, StreamMapping, expected_counts, probe, remux, snapshot, verify};
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
