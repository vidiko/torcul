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

#[cfg(test)]
mod io_tests;

pub const IO_TIMEOUT: Duration = Duration::from_secs(600);
const HASH_CHUNK_BYTES: u64 = 1024 * 1024;

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

/// Read-only opening refuses devices, sockets, symlinks and FIFOs before opening.
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
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime_ns: u128,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileStat {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime_ns: u128,
    pub nlink: u64,
}

pub fn same_device(source_dev: u64, staging_dev: u64) -> Result<()> {
    if source_dev != staging_dev {
        return Err(Error::Invalid(format!(
            "staging directory device ({staging_dev}) differs from source device ({source_dev})"
        )));
    }
    Ok(())
}

pub fn path_dev(path: &Path, deadline: Deadline) -> Result<u64> {
    let meta = fs(
        "stat path device",
        deadline.run(|| std::fs::symlink_metadata(path)),
    )?;
    metadata_dev_ino_nlink(&meta).map(|(dev, _, _)| dev)
}

pub fn fingerprint(path: &Path) -> Result<Fingerprint> {
    let deadline = Deadline::new();
    let mut file = fs("open fingerprint source", open_regular(path, deadline))?;
    let before = stat_file(&file, deadline)?;
    if before.nlink > 1 {
        return Err(Error::Invalid("source has multiple hard links".into()));
    }
    let sha256 = hash_whole_file(&mut file, before.size, deadline)?;
    let after = stat_file(&file, deadline)?;
    ensure_unchanged_stat(before, after)?;
    Ok(Fingerprint {
        dev: before.dev,
        ino: before.ino,
        size: before.size,
        mtime_ns: before.mtime_ns,
        sha256,
    })
}

pub(crate) fn ensure_unchanged_stat(before: FileStat, after: FileStat) -> Result<()> {
    if (before.dev, before.ino, before.size, before.mtime_ns)
        != (after.dev, after.ino, after.size, after.mtime_ns)
    {
        return Err(Error::Invalid("source changed while hashing".into()));
    }
    Ok(())
}

fn stat_file(file: &File, deadline: Deadline) -> Result<FileStat> {
    let meta = fs("fstat fingerprint file", deadline.run(|| file.metadata()))?;
    let (dev, ino, nlink) = metadata_dev_ino_nlink(&meta)?;
    let mtime_ns = fs("read file mtime", meta.modified())?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Invalid(format!("mtime before epoch: {e}")))?
        .as_nanos();
    Ok(FileStat {
        dev,
        ino,
        size: meta.len(),
        mtime_ns,
        nlink,
    })
}

#[cfg(unix)]
fn metadata_dev_ino_nlink(meta: &std::fs::Metadata) -> Result<(u64, u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Ok((meta.dev(), meta.ino(), meta.nlink()))
}

#[cfg(not(unix))]
fn metadata_dev_ino_nlink(_meta: &std::fs::Metadata) -> Result<(u64, u64, u64)> {
    Err(Error::Invalid(
        "POSIX device, inode and link-count fingerprinting requires Unix".into(),
    ))
}

fn hash_whole_file(file: &mut File, size: u64, deadline: Deadline) -> Result<String> {
    let mut remaining = size;
    let mut buffer = vec![0_u8; size.min(HASH_CHUNK_BYTES) as usize];
    let mut hash = Sha256::new();
    let max_reads = size.saturating_add(1).max(1);
    for _ in 0..max_reads {
        if remaining == 0 {
            break;
        }
        let target = remaining.min(buffer.len() as u64) as usize;
        let read = fs(
            "read fingerprint bytes",
            deadline.run(|| file.read(&mut buffer[..target])),
        )?;
        if read == 0 {
            return Err(Error::Invalid("source changed while hashing".into()));
        }
        hash.update(&buffer[..read]);
        remaining -= read as u64;
    }
    if remaining != 0 {
        return Err(Error::Invalid("fingerprint byte bound exceeded".into()));
    }
    let mut extra = [0_u8; 1];
    let trailing = fs(
        "check trailing bytes",
        deadline.run(|| file.read(&mut extra)),
    )?;
    if trailing != 0 {
        return Err(Error::Invalid("source changed while hashing".into()));
    }
    Ok(format!("sha256:{}", hex(&hash.finalize())))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
