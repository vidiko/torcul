// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod support;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};
use support::*;
use torcul_media::{probe, remux, verify};
use torcul_policy::{TitleMeta, plan};

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
