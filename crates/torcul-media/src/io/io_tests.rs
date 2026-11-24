// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use super::*;
use std::fs;

fn is_sha256_prefixed_hex(value: &str) -> bool {
    let Some(hex_part) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex_part.len() == 64
        && hex_part
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[test]
fn fingerprint_mid_file_change_with_restored_mtime_mismatches() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.bin");
    let size = 3 * 1024 * 1024 + 8192;
    let mut bytes = vec![0x3c_u8; size];
    fs::write(&path, &bytes).unwrap();
    let original_mtime = fs::metadata(&path).unwrap().modified().unwrap();

    let initial = fingerprint(&path).unwrap();
    assert_eq!(initial.size, size as u64);
    assert!(
        is_sha256_prefixed_hex(&initial.sha256),
        "{}",
        initial.sha256
    );
    assert_eq!(initial, fingerprint(&path).unwrap());

    bytes[size / 2] ^= 0x01;
    fs::write(&path, &bytes).unwrap();
    File::open(&path)
        .unwrap()
        .set_modified(original_mtime)
        .unwrap();

    let mutated = fingerprint(&path).unwrap();
    assert_eq!(mutated.size, initial.size);
    assert_eq!(mutated.mtime_ns, initial.mtime_ns);
    assert_ne!(mutated.sha256, initial.sha256);
    assert_ne!(mutated, initial);
}

#[test]
fn fingerprint_mtime_only_change_and_hard_link_refused() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.bin");
    fs::write(&path, b"torcul-fingerprint-payload").unwrap();

    let before = fingerprint(&path).unwrap();
    assert!(is_sha256_prefixed_hex(&before.sha256));

    let later = fs::metadata(&path).unwrap().modified().unwrap() + Duration::from_secs(2);
    File::open(&path).unwrap().set_modified(later).unwrap();
    let touched = fingerprint(&path).unwrap();
    assert_eq!(before.sha256, touched.sha256);
    assert_ne!(before.mtime_ns, touched.mtime_ns);
    assert_ne!(before, touched);

    let link = temp.path().join("sample.hardlink");
    fs::hard_link(&path, &link).unwrap();
    let err = fingerprint(&path).unwrap_err();
    assert!(
        err.to_string().contains("hard links"),
        "unexpected error: {err}"
    );
}

#[test]
fn same_device_and_stat_race_checks() {
    assert!(same_device(42, 42).is_ok());
    let err = same_device(42, 43).unwrap_err();
    assert!(err.to_string().contains("differs from source device"));

    let base = FileStat {
        dev: 10,
        ino: 20,
        size: 300,
        mtime_ns: 4000,
        nlink: 1,
    };
    assert!(ensure_unchanged_stat(base, base).is_ok());
    for mutated in [
        FileStat { dev: 11, ..base },
        FileStat { ino: 21, ..base },
        FileStat { size: 301, ..base },
        FileStat {
            mtime_ns: 4001,
            ..base
        },
    ] {
        let err = ensure_unchanged_stat(base, mutated).unwrap_err();
        assert_eq!(err.to_string(), "source changed while hashing");
    }
}
