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
use torcul_media::StreamCounts;
use wait_timeout::ChildExt;

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
}

/// Generate and cache the track fixtures in `CARGO_TARGET_TMPDIR` (`{CARGO_CRATE_NAME}-fixtures`).
///
/// `cargo test` runs one process per test binary, so `OnceLock` initializes the
/// binary's fixture directory once per run; per-test-process runners (`nextest`)
/// are unsupported here.
pub fn fixtures() -> &'static Fixtures {
    static FIXTURES: OnceLock<Fixtures> = OnceLock::new();
    FIXTURES.get_or_init(|| {
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
        let manifest =
            serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        Fixtures {
            dir: FixtureDir(dir),
            manifest,
        }
    })
}

pub fn reset_fixture_dir(dir: &Path) {
    if let Ok(meta) = fs::symlink_metadata(dir) {
        if meta.file_type().is_dir() {
            fs::remove_dir_all(dir).unwrap();
        } else {
            fs::remove_file(dir).unwrap();
        }
    }
    fs::create_dir_all(dir).unwrap();
}

pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub fn execute(command: &mut Command, timeout: Duration) -> Output {
    let logs = tempfile::tempdir().unwrap();
    let stdout_path = logs.path().join("stdout");
    let mut out = execute_to(command, &stdout_path, timeout);
    out.stdout = fs::read_to_string(stdout_path).unwrap();
    out
}

pub fn execute_to(command: &mut Command, stdout_path: &Path, timeout: Duration) -> Output {
    let logs = tempfile::tempdir().unwrap();
    let stderr_path = logs.path().join("stderr");
    let stdout = fs::File::create(stdout_path).unwrap();
    let stderr = fs::File::create(&stderr_path).unwrap();
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
    Output {
        success: status.success(),
        stdout: String::new(),
        stderr: String::from_utf8_lossy(&fs::read(stderr_path).unwrap()).into_owned(),
    }
}

pub fn copy_fixture(name: &str) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join(name);
    fs::copy(fixtures().dir.join(name), &copy).unwrap();
    (dir, copy)
}
