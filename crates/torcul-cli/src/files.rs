// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use torcul_media::io::{Deadline, open_regular};

const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let deadline = Deadline::new();
    let file = open_regular(path, deadline).with_context(|| format!("open {}", path.display()))?;
    let mut bytes = Vec::new();
    deadline.run(|| file.take(MAX_JSON_BYTES + 1).read_to_end(&mut bytes))?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        bail!("JSON exceeds {MAX_JSON_BYTES} bytes");
    }
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}
pub fn write_json(path: &Path, data: &impl Serialize) -> Result<()> {
    let deadline = Deadline::new();
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = deadline.run(|| tempfile::NamedTempFile::new_in(parent))?;
    let bytes = serde_json::to_vec_pretty(data)?;
    deadline.run(|| temp.write_all(&bytes))?;
    deadline.run(|| temp.as_file().sync_all())?;
    deadline.check()?;
    temp.persist(path)
        .with_context(|| format!("persist {}", path.display()))?;
    deadline.check()?;
    Ok(())
}
pub fn expand(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let deadline = Deadline::new();
    let mut files = Vec::new();
    for path in paths {
        let meta = deadline
            .run(|| fs::symlink_metadata(path))
            .with_context(|| format!("stat {}", path.display()))?;
        if meta.is_dir() {
            let entries = deadline.run(|| fs::read_dir(path))?;
            for (index, entry) in entries.take(1_000_001).enumerate() {
                let entry = deadline.run(|| entry)?;
                if index >= 1_000_000 {
                    bail!("directory file bound exceeded");
                }
                if deadline.run(|| entry.file_type())?.is_file() && media_extension(&entry.path()) {
                    files.push(entry.path());
                }
            }
        } else {
            files.push(path.clone());
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}
fn media_extension(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "mkv"
                | "mp4"
                | "m4v"
                | "m4a"
                | "mov"
                | "webm"
                | "avi"
                | "ts"
                | "m2ts"
                | "flac"
                | "mp3"
                | "ogg"
                | "wav"
        )
    })
}
pub fn canonical(path: &Path) -> Result<PathBuf> {
    Deadline::new()
        .run(|| path.canonicalize())
        .with_context(|| format!("canonicalize {}", path.display()))
}
