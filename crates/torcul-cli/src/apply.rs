// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::staging;
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use torcul_media::io::{Deadline, fingerprint, open_regular};

const COPY_CHUNK_BYTES: usize = 1024 * 1024;
/// No backup: operator opts in by invoking apply explicitly.
pub fn apply(src: &Path, staging: &Path) -> Result<()> {
    let (output, _, record) = staging::load(src, staging)?;
    ensure!(
        record
            .verification
            .as_ref()
            .is_some_and(|r| r.pass && r.failures.is_empty()),
        "passing verify required before apply"
    );
    let verified = record
        .verified_staging_sha256
        .context("missing verified staging digest")?;
    ensure!(
        torcul_media::io::file_sha256(&output)? == verified,
        "staging changed after verify"
    );
    ensure!(
        fingerprint(src)? == record.fingerprint,
        "source fingerprint changed"
    );
    let deadline = Deadline::new();
    let parent = src
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = deadline.run(|| tempfile::NamedTempFile::new_in(parent))?;
    copy(&output, temp.as_file_mut(), deadline)?;
    deadline.run(|| {
        temp.as_file()
            .set_permissions(fs::metadata(src)?.permissions())
    })?;
    deadline.run(|| temp.as_file().sync_all())?;
    ensure!(
        torcul_media::io::file_sha256(temp.path())? == verified,
        "replacement copy differs from verified staging"
    );
    ensure!(
        fingerprint(src)? == record.fingerprint,
        "source changed during copy"
    );
    deadline.check()?;
    temp.persist(src).context("atomic source replacement")?;
    sync_directory(parent, deadline)?;
    Ok(())
}
fn copy(src: &Path, dst: &mut fs::File, deadline: Deadline) -> Result<()> {
    let mut source = open_regular(src, deadline)?;
    let size = deadline.run(|| source.metadata())?.len();
    let mut remaining = size;
    let mut buffer = vec![0; COPY_CHUNK_BYTES];
    for _ in 0..=size / COPY_CHUNK_BYTES as u64 {
        if remaining == 0 {
            break;
        }
        let amount = remaining.min(COPY_CHUNK_BYTES as u64) as usize;
        deadline.run(|| source.read_exact(&mut buffer[..amount]))?;
        deadline.run(|| dst.write_all(&buffer[..amount]))?;
        remaining -= amount as u64;
    }
    ensure!(remaining == 0, "copy byte bound exceeded");
    Ok(())
}
fn sync_directory(parent: &Path, deadline: Deadline) -> Result<()> {
    #[cfg(unix)]
    {
        deadline
            .run(|| fs::File::open(parent)?.sync_all())
            .context("fsync source directory")?;
    }
    #[cfg(not(unix))]
    {
        let _ = (parent, deadline);
    } // Directory fsync unavailable through std on Windows.
    Ok(())
}
