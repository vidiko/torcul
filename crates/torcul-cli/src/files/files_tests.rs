// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use super::*;
use std::time::Duration;

#[test]
fn expand_empty_directory_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    let expanded = expand(&[dir.path().to_path_buf()]).unwrap();
    assert!(expanded.is_empty());
}

#[test]
fn expand_filters_and_sorts_media_without_recursing() {
    let dir = tempfile::tempdir().unwrap();
    let second = dir.path().join("z_second.mkv");
    let first = dir.path().join("a_first.MP4");
    let ignored = dir.path().join("notes.txt");
    fs::write(&second, b"mkv").unwrap();
    fs::write(&first, b"mp4").unwrap();
    fs::write(&ignored, b"txt").unwrap();

    let nested = dir.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("hidden.mkv"), b"nested").unwrap();

    let expanded = expand(&[dir.path().to_path_buf()]).unwrap();
    assert_eq!(expanded, vec![first, second]);
}

#[test]
fn expand_exhaustion_advance_checks_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let entries = fs::read_dir(dir.path()).unwrap();
    let expired = Deadline::with_timeout(Duration::ZERO).unwrap();
    let mut files = Vec::new();
    let err = collect_dir_entries(entries, expired, &mut files).unwrap_err();
    assert!(
        err.to_string().contains("deadline exceeded"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn expand_missing_path_fails_with_path_context() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.mkv");
    let err = expand(std::slice::from_ref(&missing)).unwrap_err();
    assert!(
        err.to_string().contains(&missing.display().to_string()),
        "{err:#}"
    );
}
