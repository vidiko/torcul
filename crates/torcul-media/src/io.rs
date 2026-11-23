// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{Error, Result, error::fs};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    time::{Duration, Instant, UNIX_EPOCH},
};

pub const IO_TIMEOUT: Duration = Duration::from_secs(600);
pub const FINGERPRINT_WINDOW: u64 = 1024 * 1024;

/// Local regular-file operations check this deadline before and after syscalls.
/// Kernel stalls inside a regular-file syscall cannot be interrupted portably.
#[derive(Clone, Copy)]
pub struct Deadline(Instant);
impl Default for Deadline {
    fn default() -> Self {
        Self::new()
    }
}
impl Deadline {
    pub fn new() -> Self {
        Self(Instant::now() + IO_TIMEOUT)
    }
    pub fn with_timeout(timeout: Duration) -> std::io::Result<Self> {
        Instant::now()
            .checked_add(timeout)
            .map(Self)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "deadline exceeds clock range",
                )
            })
    }
    /// Error cleanup needs its own deadline after an operation times out.
    pub fn remove_partial(self, path: &Path) -> std::io::Result<()> {
        Self::new().run(|| std::fs::remove_file(path))
    }
    pub fn expired(self) -> bool {
        Instant::now() >= self.0
    }
    pub fn check(self) -> std::io::Result<()> {
        if self.expired() {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "file I/O deadline exceeded",
            ))
        } else {
            Ok(())
        }
    }
    pub fn run<T>(self, operation: impl FnOnce() -> std::io::Result<T>) -> std::io::Result<T> {
        self.check()?;
        let result = operation()?;
        self.check()?;
        Ok(result)
    }
}

/// Read-only opening refuses devices, sockets and FIFOs before opening.
pub fn open_regular(path: &Path, deadline: Deadline) -> std::io::Result<File> {
    let meta = deadline.run(|| std::fs::symlink_metadata(path))?;
    if !meta.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "expected regular file (no symlinks)",
        ));
    }
    deadline.run(|| File::open(path))
}

pub(crate) struct TimedFile {
    pub file: File,
    pub deadline: Deadline,
}
impl Read for TimedFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.deadline.run(|| self.file.read(buffer))
    }
}
impl Write for TimedFile {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.deadline.run(|| self.file.write(bytes))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.deadline.run(|| self.file.flush())
    }
}
impl Seek for TimedFile {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.deadline.run(|| self.file.seek(position))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub size: u64,
    pub mtime_ns: u128,
    pub sha256: String,
}
pub fn fingerprint(path: &Path) -> Result<Fingerprint> {
    let deadline = Deadline::new();
    let mut file = fs("open fingerprint source", open_regular(path, deadline))?;
    let meta = fs("stat fingerprint source", deadline.run(|| file.metadata()))?;
    let mtime_ns = fs("read source mtime", meta.modified())?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Invalid(format!("mtime before epoch: {e}")))?
        .as_nanos();
    let mut hash = Sha256::new();
    let size = meta.len();
    hash_window(
        &mut file,
        0,
        size.min(FINGERPRINT_WINDOW),
        deadline,
        &mut hash,
    )?;
    hash_window(
        &mut file,
        size.saturating_sub(FINGERPRINT_WINDOW),
        size.min(FINGERPRINT_WINDOW),
        deadline,
        &mut hash,
    )?;
    let after = fs(
        "restat fingerprint source",
        deadline.run(|| file.metadata()),
    )?;
    if meta.len() != after.len()
        || fs("read final mtime", after.modified())? != fs("read initial mtime", meta.modified())?
    {
        return Err(Error::Invalid("source changed during fingerprint".into()));
    }
    Ok(Fingerprint {
        size,
        mtime_ns,
        sha256: hex(&hash.finalize()),
    })
}

fn hash_window(
    file: &mut File,
    offset: u64,
    length: u64,
    deadline: Deadline,
    hash: &mut Sha256,
) -> Result<()> {
    fs(
        "seek fingerprint window",
        deadline.run(|| file.seek(SeekFrom::Start(offset))),
    )?;
    let mut buffer = vec![0; length as usize];
    fs(
        "read fingerprint window",
        deadline.run(|| file.read_exact(&mut buffer)),
    )?;
    hash.update(buffer);
    Ok(())
}

/// Whole-file digest binds a passing verification to exact staged bytes.
pub fn file_sha256(path: &Path) -> Result<String> {
    let deadline = Deadline::new();
    let mut file = fs("open whole-file digest", open_regular(path, deadline))?;
    let size = fs("stat digest input", deadline.run(|| file.metadata()))?.len();
    let mut remaining = size;
    let mut buffer = vec![0; FINGERPRINT_WINDOW as usize];
    let mut hash = Sha256::new();
    for _ in 0..=size / FINGERPRINT_WINDOW {
        if remaining == 0 {
            break;
        }
        let n = remaining.min(FINGERPRINT_WINDOW) as usize;
        fs(
            "read whole-file digest",
            deadline.run(|| file.read_exact(&mut buffer[..n])),
        )?;
        hash.update(&buffer[..n]);
        remaining -= n as u64;
    }
    if remaining != 0 {
        return Err(Error::Invalid("digest byte bound exceeded".into()));
    }
    Ok(hex(&hash.finalize()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
