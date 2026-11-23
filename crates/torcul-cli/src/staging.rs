// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{
    files,
    report::{self, Metadata},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use torcul_media::{
    VerifyReport,
    io::{Deadline, Fingerprint, fingerprint},
};
use torcul_policy::{FileFacts, Plan, TitleMeta};

#[derive(Serialize, Deserialize)]
pub struct Record {
    version: u32,
    pub source: PathBuf,
    pub fingerprint: Fingerprint,
    pub facts: FileFacts,
    pub meta: TitleMeta,
    pub plan: Plan,
    pub verification: Option<VerifyReport>,
    pub verified_staging_sha256: Option<String>,
}
pub fn paths(src: &Path, staging: &Path) -> Result<(PathBuf, PathBuf)> {
    let name = src.file_name().context("source has no basename")?;
    let output = staging.join(name);
    let mut sidecar = output.as_os_str().to_os_string();
    sidecar.push(".torcul.json");
    Ok((output, PathBuf::from(sidecar)))
}
pub fn prepare(src: &Path, staging: &Path, meta: &Metadata) -> Result<()> {
    let title = report::title(src, meta)?.clone();
    let before = fingerprint(src)?;
    let facts = torcul_media::probe(src)?;
    let plan = torcul_policy::plan(&facts, &title);
    if !plan.needs_remux {
        return Ok(());
    }
    let deadline = Deadline::new();
    deadline.run(|| std::fs::create_dir_all(staging))?;
    let (output, sidecar) = paths(src, staging)?;
    ensure!(
        !deadline.run(|| sidecar.try_exists())?,
        "staging sidecar already exists"
    );
    let record = Record {
        version: 1,
        source: files::canonical(src)?,
        fingerprint: before,
        facts,
        meta: title,
        plan,
        verification: None,
        verified_staging_sha256: None,
    };
    torcul_media::remux(src, &record.plan, &output)?;
    let result = save_new(src, &sidecar, &record);
    if result.is_err() {
        deadline
            .remove_partial(&output)
            .context("remove unrecorded staging")?;
    }
    result
}
fn save_new(src: &Path, sidecar: &Path, record: &Record) -> Result<()> {
    ensure!(
        fingerprint(src)? == record.fingerprint,
        "source changed during remux"
    );
    files::write_json(sidecar, record)
}
pub fn load(src: &Path, staging: &Path) -> Result<(PathBuf, PathBuf, Record)> {
    let (output, sidecar) = paths(src, staging)?;
    let record: Record = files::read_json(&sidecar)?;
    ensure!(record.version == 1, "unsupported sidecar version");
    ensure!(
        record.source == files::canonical(src)?,
        "source path does not match sidecar"
    );
    ensure!(
        torcul_policy::plan(&record.facts, &record.meta) == record.plan,
        "saved plan is inconsistent"
    );
    Ok((output, sidecar, record))
}
pub fn verify(src: &Path, staging: &Path) -> Result<()> {
    let (output, sidecar, mut record) = load(src, staging)?;
    record.verification = None;
    record.verified_staging_sha256 = None;
    files::write_json(&sidecar, &record)?;
    let (report, digest) = match verified_report(src, &output, &record) {
        Ok(value) => value,
        Err(error) => (VerifyReport::failed(format!("{error:#}")), None),
    };
    let pass = report.pass;
    let failures = report.failures.clone();
    record.verification = Some(report);
    record.verified_staging_sha256 = digest;
    files::write_json(&sidecar, &record)?;
    if !pass {
        bail!("staging verification failed: {failures:?}");
    }
    Ok(())
}
fn verified_report(
    src: &Path,
    output: &Path,
    record: &Record,
) -> Result<(VerifyReport, Option<String>)> {
    ensure!(
        fingerprint(src)? == record.fingerprint,
        "source changed since remux"
    );
    let before = torcul_media::io::file_sha256(output)?;
    let report = torcul_media::verify(src, &record.plan, output)?;
    ensure!(
        fingerprint(src)? == record.fingerprint,
        "source changed during verify"
    );
    ensure!(
        before == torcul_media::io::file_sha256(output)?,
        "staging changed during verify"
    );
    let digest = report.pass.then_some(before);
    Ok((report, digest))
}
