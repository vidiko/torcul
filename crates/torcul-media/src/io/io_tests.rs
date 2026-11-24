// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use super::*;
use std::fs;

#[cfg(unix)]
fn is_sha256_prefixed_hex(value: &str) -> bool {
    let Some(hex_part) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex_part.len() == 64
        && hex_part
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(unix)]
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

#[cfg(unix)]
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
        FileStat { nlink: 2, ..base },
        FileStat { nlink: 0, ..base },
    ] {
        let err = ensure_unchanged_stat(base, mutated).unwrap_err();
        assert_eq!(err.to_string(), "source changed while hashing");
    }
}

#[cfg(not(unix))]
#[test]
fn fingerprint_requires_unix_on_non_unix() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("regular.bin");
    fs::write(&path, b"torcul-non-unix").unwrap();
    let err = fingerprint(&path).unwrap_err();
    assert!(
        err.to_string().contains("requires Unix"),
        "unexpected error: {err}"
    );
}

#[cfg(unix)]
fn make_fifo(path: &Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    use wait_timeout::ChildExt;
    let mut child = Command::new("mkfifo")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    match child.wait_timeout(Duration::from_secs(10))? {
        Some(status) if status.success() => Ok(()),
        Some(status) => Err(std::io::Error::other(format!("mkfifo failed: {status}"))),
        None => {
            child.kill()?;
            child.wait()?;
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "mkfifo timed out",
            ))
        }
    }
}

#[cfg(unix)]
#[test]
fn open_regular_refuses_symlink_dir_socket_and_fifo_without_blocking() {
    use std::os::unix::{fs::symlink, net::UnixListener};
    let temp = tempfile::tempdir().unwrap();
    let regular = temp.path().join("regular.bin");
    fs::write(&regular, b"hello").unwrap();
    let deadline = Deadline::new();

    let mut opened = open_regular(&regular, deadline).unwrap();
    let mut buf = Vec::new();
    opened.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, b"hello");

    let link = temp.path().join("link.bin");
    symlink(&regular, &link).unwrap();
    assert!(open_regular(&link, deadline).is_err());
    assert!(open_regular_fd(&link, deadline).is_err());

    let dir = temp.path().join("subdir");
    fs::create_dir(&dir).unwrap();
    assert!(open_regular(&dir, deadline).is_err());
    let dir_err = open_regular_fd(&dir, deadline).unwrap_err();
    assert_eq!(dir_err.kind(), std::io::ErrorKind::InvalidInput);

    let sock = temp.path().join("test.sock");
    let _listener = UnixListener::bind(&sock).unwrap();
    assert!(open_regular(&sock, deadline).is_err());
    assert!(open_regular_fd(&sock, deadline).is_err());

    let fifo = temp.path().join("test.fifo");
    make_fifo(&fifo).unwrap();
    let started = Instant::now();
    assert!(open_regular(&fifo, deadline).is_err());
    let fifo_err = open_regular_fd(&fifo, deadline).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(fifo_err.kind(), std::io::ErrorKind::InvalidInput);
}

#[cfg(unix)]
#[test]
fn path_dev_follows_directory_symlinks_and_rejects_missing() {
    use std::os::unix::fs::{MetadataExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target-dir");
    fs::create_dir(&target).unwrap();
    let link = temp.path().join("link-dir");
    symlink(&target, &link).unwrap();

    let deadline = Deadline::new();
    let expected = fs::metadata(&target).unwrap().dev();
    assert_eq!(path_dev(&link, deadline).unwrap(), expected);

    let manifest_link = temp.path().join("manifest-dir-link");
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    symlink(manifest_dir, &manifest_link).unwrap();
    assert_eq!(
        path_dev(&manifest_link, deadline).unwrap(),
        fs::metadata(manifest_dir).unwrap().dev()
    );

    assert!(path_dev(&temp.path().join("missing"), deadline).is_err());
    let dangling = temp.path().join("dangling");
    symlink(temp.path().join("gone"), &dangling).unwrap();
    assert!(path_dev(&dangling, deadline).is_err());

    let expired = Deadline::with_timeout(Duration::ZERO).unwrap();
    assert!(path_dev(&target, expired).is_err());
}
