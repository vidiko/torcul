// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
#![allow(dead_code)]

use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::Duration,
};
use tempfile::TempDir;
use torcul_media::{StreamCounts, io::Deadline};
use wait_timeout::ChildExt;

const LOCK_POLL: Duration = Duration::from_millis(100);
const LOCK_POLLS: u32 = 18_000;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub fixtures: Vec<Row>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FixtureCounts {
    pub source: StreamCounts,
    pub output: StreamCounts,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Row {
    pub name: String,
    pub original: Option<String>,
    pub anime: bool,
    pub expect: Vec<usize>,
    pub warnings: Vec<String>,
    pub needs_remux: bool,
    pub chapters: usize,
    pub counts: FixtureCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureDir(pub PathBuf);

impl FixtureDir {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl std::ops::Deref for FixtureDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for FixtureDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

pub struct Fixtures {
    pub dir: FixtureDir,
    pub manifest: Manifest,
    _lock: fs::File,
}

pub fn fixture_lock_path() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("{}-fixtures.lock", env!("CARGO_CRATE_NAME")))
}

/// Exclusive non-blocking lock within `deadline`: `true` = lock taken, `false` = held elsewhere.
pub fn try_lock_within(file: &fs::File, deadline: Deadline) -> std::io::Result<bool> {
    deadline.run(|| match file.try_lock() {
        Ok(()) => Ok(true),
        Err(fs::TryLockError::WouldBlock) => Ok(false),
        Err(fs::TryLockError::Error(err)) => Err(err),
    })
}

fn acquire_fixture_lock(lock_path: &Path) -> fs::File {
    let deadline = Deadline::new();
    let file = deadline
        .run(|| {
            fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(lock_path)
        })
        .unwrap();
    for _ in 0..LOCK_POLLS {
        match try_lock_within(&file, Deadline::new()) {
            Ok(true) => return file,
            Ok(false) => std::thread::sleep(LOCK_POLL),
            Err(err) => {
                panic!("failed to lock {}: {err}", lock_path.display());
            }
        }
    }
    panic!(
        "timed out waiting for fixture lock {} after {} polls ({:?})",
        lock_path.display(),
        LOCK_POLLS,
        LOCK_POLL * LOCK_POLLS
    );
}

/// Generate and cache the track fixtures in `CARGO_TARGET_TMPDIR` (`{CARGO_CRATE_NAME}-fixtures`).
///
/// An exclusive cross-process file lock (`{CARGO_CRATE_NAME}-fixtures.lock`) held
/// for the process lifetime serializes concurrent test processes on one target
/// directory; per-test-process runners wait on the lock.
pub fn fixtures() -> &'static Fixtures {
    static FIXTURES: OnceLock<Fixtures> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let lock = acquire_fixture_lock(&fixture_lock_path());
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("{}-fixtures", env!("CARGO_CRATE_NAME")));
        reset_fixture_dir(&dir);
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixtures/track_fixtures.py");
        let result = execute(
            Command::new("python3").arg("-I").arg(script).arg(&dir),
            Duration::from_secs(600),
        );
        assert!(
            result.success,
            "fixture generation requires python3 and ffmpeg: {}",
            result.stderr
        );
        let deadline = Deadline::new();
        let raw_manifest = deadline
            .run(|| fs::read(dir.join("manifest.json")))
            .unwrap();
        let manifest = serde_json::from_slice(&raw_manifest).unwrap();
        Fixtures {
            dir: FixtureDir(dir),
            manifest,
            _lock: lock,
        }
    })
}

pub fn reset_fixture_dir(dir: &Path) {
    let deadline = Deadline::new();
    match deadline.run(|| fs::symlink_metadata(dir)) {
        Ok(meta) if meta.file_type().is_dir() => {
            deadline.run(|| fs::remove_dir_all(dir)).unwrap();
        }
        Ok(_) => {
            deadline.run(|| fs::remove_file(dir)).unwrap();
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => panic!("stat {}: {err}", dir.display()),
    }
    deadline.run(|| fs::create_dir_all(dir)).unwrap();
}

pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub fn execute(command: &mut Command, timeout: Duration) -> Output {
    let deadline = Deadline::new();
    let logs = deadline.run(tempfile::tempdir).unwrap();
    let stdout_path = logs.path().join("stdout");
    let mut out = execute_to(command, &stdout_path, timeout);
    out.stdout = deadline.run(|| fs::read_to_string(stdout_path)).unwrap();
    out
}

pub fn execute_to(command: &mut Command, stdout_path: &Path, timeout: Duration) -> Output {
    let deadline = Deadline::new();
    let logs = deadline.run(tempfile::tempdir).unwrap();
    let stderr_path = logs.path().join("stderr");
    let stdout = deadline.run(|| fs::File::create(stdout_path)).unwrap();
    let stderr = deadline.run(|| fs::File::create(&stderr_path)).unwrap();
    command.stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    let mut child = command
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {command:?}: {e}; required tool missing"));
    let status = match child.wait_timeout(timeout).unwrap() {
        Some(status) => status,
        None => {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("child timeout: {command:?}");
        }
    };
    let stderr_bytes = deadline.run(|| fs::read(stderr_path)).unwrap();
    Output {
        success: status.success(),
        stdout: String::new(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
    }
}

pub fn copy_fixture(name: &str) -> (TempDir, PathBuf) {
    let fx = fixtures();
    let deadline = Deadline::new();
    let dir = deadline.run(tempfile::tempdir).unwrap();
    let copy = dir.path().join(name);
    deadline.run(|| fs::copy(fx.dir.join(name), &copy)).unwrap();
    (dir, copy)
}
