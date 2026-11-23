// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::Duration,
};
use tempfile::TempDir;
use wait_timeout::ChildExt;

#[derive(Deserialize)]
pub struct Manifest {
    pub fixtures: Vec<Row>,
}
#[derive(Deserialize)]
pub struct Row {
    pub name: String,
    pub original: Option<String>,
    pub anime: bool,
    pub expect: Vec<usize>,
    pub warnings: Vec<String>,
    pub needs_remux: bool,
}
pub struct Fixtures {
    pub dir: TempDir,
    pub manifest: Manifest,
}
pub fn fixtures() -> &'static Fixtures {
    static FIXTURES: OnceLock<Fixtures> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixtures/track_fixtures.py");
        let result = execute(
            Command::new("python3")
                .arg("-I")
                .arg(script)
                .arg(dir.path()),
            Duration::from_secs(600),
        );
        assert!(
            result.success,
            "fixture generation requires python3 and ffmpeg: {}",
            result.stderr
        );
        let manifest =
            serde_json::from_slice(&fs::read(dir.path().join("manifest.json")).unwrap()).unwrap();
        Fixtures { dir, manifest }
    })
}
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}
pub fn execute(command: &mut Command, timeout: Duration) -> Output {
    let logs = tempfile::tempdir().unwrap();
    let stdout_path = logs.path().join("stdout");
    let stderr_path = logs.path().join("stderr");
    let stdout = fs::File::create(&stdout_path).unwrap();
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
        stdout: fs::read_to_string(stdout_path).unwrap(),
        stderr: fs::read_to_string(stderr_path).unwrap(),
    }
}
fn binary() -> PathBuf {
    let executable = std::env::current_exe().unwrap();
    let debug = executable
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "debug"))
        .unwrap();
    let path = debug.join(format!("torcul{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "cargo build required before CLI integration tests: {}",
        path.display()
    );
    path
}
pub fn cli(args: &[&std::ffi::OsStr]) -> Output {
    execute(Command::new(binary()).args(args), Duration::from_secs(600))
}
pub fn command(name: &str, meta: Option<&Path>, staging: Option<&Path>, file: &Path) -> Output {
    let mut args = vec![std::ffi::OsStr::new(name)];
    if let Some(meta) = meta {
        args.extend([std::ffi::OsStr::new("--meta"), meta.as_os_str()]);
    }
    if let Some(staging) = staging {
        args.extend([std::ffi::OsStr::new("--staging"), staging.as_os_str()]);
    }
    args.push(file.as_os_str());
    cli(&args)
}
pub fn copy_fixture(name: &str) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join(name);
    fs::copy(fixtures().dir.path().join(name), &copy).unwrap();
    (dir, copy)
}
