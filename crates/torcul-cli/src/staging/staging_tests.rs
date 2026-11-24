// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use super::*;
use std::{collections::BTreeSet, fs, time::Duration};
use torcul_policy::{TrackInfo, TrackKind};

fn sample_record(src: &Path, fp: Fingerprint) -> Result<Record> {
    let facts = FileFacts {
        container: "matroska".into(),
        chapters: 0,
        tracks: vec![
            TrackInfo {
                index: 0,
                kind: TrackKind::Video,
                codec: "h264".into(),
                language: None,
                title: None,
                dispositions: BTreeSet::new(),
                attached_pic: false,
            },
            TrackInfo {
                index: 1,
                kind: TrackKind::Audio,
                codec: "aac".into(),
                language: Some("fra".into()),
                title: None,
                dispositions: BTreeSet::new(),
                attached_pic: false,
            },
            TrackInfo {
                index: 2,
                kind: TrackKind::Audio,
                codec: "aac".into(),
                language: Some("eng".into()),
                title: None,
                dispositions: BTreeSet::new(),
                attached_pic: false,
            },
        ],
    };
    let meta = TitleMeta {
        original: Some("en".into()),
        anime: false,
    };
    let plan = torcul_policy::plan(&facts, &meta)?;
    Ok(Record {
        version: 1,
        source: files::canonical(src)?,
        fingerprint: fp,
        facts,
        meta,
        plan,
        verification: None,
        verified_staging: None,
    })
}

#[test]
fn post_sidecar_failure_removes_output_and_sidecar_preserving_unrelated() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("movie.mkv");
    fs::write(&src, b"source-bytes").unwrap();
    let fp = fingerprint(&src).unwrap();
    let record = sample_record(&src, fp).unwrap();

    let staging = temp.path().join("staging");
    fs::create_dir(&staging).unwrap();
    let unrelated = staging.join("keep.txt");
    fs::write(&unrelated, b"pre-existing").unwrap();

    let (output, sidecar) = paths(&src, &staging).unwrap();
    fs::write(&output, b"staged-remux-bytes").unwrap();
    assert!(!sidecar.exists());

    let expired = Deadline::with_timeout(Duration::ZERO).unwrap();
    let err = commit_staged(&src, &output, &sidecar, &record, expired).unwrap_err();
    assert!(
        err.to_string().contains("staging completion deadline"),
        "{err:#}"
    );
    assert!(!output.exists(), "staged output must be removed");
    assert!(!sidecar.exists(), "persisted sidecar must be removed");
    assert_eq!(fs::read(&unrelated).unwrap(), b"pre-existing");
}

#[test]
fn commit_staged_success_retains_output_sidecar_and_unrelated() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("movie.mkv");
    fs::write(&src, b"source-bytes").unwrap();
    let fp = fingerprint(&src).unwrap();
    let record = sample_record(&src, fp).unwrap();

    let staging = temp.path().join("staging");
    fs::create_dir(&staging).unwrap();
    let unrelated = staging.join("keep.txt");
    fs::write(&unrelated, b"pre-existing").unwrap();

    let (output, sidecar) = paths(&src, &staging).unwrap();
    fs::write(&output, b"staged-remux-bytes").unwrap();

    commit_staged(&src, &output, &sidecar, &record, Deadline::new()).unwrap();
    assert!(output.is_file());
    assert!(sidecar.is_file());
    assert_eq!(fs::read(&unrelated).unwrap(), b"pre-existing");
}

#[test]
fn pre_sidecar_source_change_removes_output_and_tolerates_missing_sidecar() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("movie.mkv");
    fs::write(&src, b"source-bytes").unwrap();
    let fp = fingerprint(&src).unwrap();
    let record = sample_record(&src, fp).unwrap();
    fs::write(&src, b"mutated-source-bytes").unwrap();

    let staging = temp.path().join("staging");
    fs::create_dir(&staging).unwrap();
    let unrelated = staging.join("keep.txt");
    fs::write(&unrelated, b"pre-existing").unwrap();

    let (output, sidecar) = paths(&src, &staging).unwrap();
    fs::write(&output, b"staged-remux-bytes").unwrap();

    let err = commit_staged(&src, &output, &sidecar, &record, Deadline::new()).unwrap_err();
    assert!(err.to_string().contains("source changed during remux"));
    assert!(!output.exists());
    assert!(!sidecar.exists());
    assert_eq!(fs::read(&unrelated).unwrap(), b"pre-existing");
}

#[test]
fn ensure_absent_rejects_pre_existing_sidecar_without_removing_it() {
    let temp = tempfile::tempdir().unwrap();
    let sidecar = temp.path().join("movie.mkv.torcul.json");
    fs::write(&sidecar, b"existing-sidecar").unwrap();

    let err =
        ensure_absent(&sidecar, Deadline::new(), "staging sidecar already exists").unwrap_err();
    assert_eq!(err.to_string(), "staging sidecar already exists");
    assert_eq!(fs::read(&sidecar).unwrap(), b"existing-sidecar");
}
