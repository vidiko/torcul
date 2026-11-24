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
    io::{Deadline, Fingerprint, fingerprint, path_dev, same_device},
};
use torcul_policy::{FileFacts, Plan, TitleMeta};

const MAX_PATH_ANCESTORS: usize = 4096;

#[derive(Serialize, Deserialize)]
pub struct Record {
    version: u32,
    pub source: PathBuf,
    pub fingerprint: Fingerprint,
    pub facts: FileFacts,
    pub meta: TitleMeta,
    pub plan: Plan,
    pub verification: Option<VerifyReport>,
    pub verified_staging: Option<Fingerprint>,
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
    let deadline = Deadline::new();
    check_staging_device(before.dev, staging, deadline)?;
    let facts = torcul_media::probe(src)?;
    let plan = torcul_policy::plan(&facts, &title)?;
    if !plan.needs_remux {
        return Ok(());
    }
    deadline.run(|| std::fs::create_dir_all(staging))?;
    same_device(before.dev, path_dev(staging, deadline)?)?;
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
        verified_staging: None,
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

fn check_staging_device(source_dev: u64, staging: &Path, deadline: Deadline) -> Result<()> {
    for ancestor in staging.ancestors().take(MAX_PATH_ANCESTORS) {
        let candidate = if ancestor.as_os_str().is_empty() {
            Path::new(".")
        } else {
            ancestor
        };
        if deadline.run(|| candidate.try_exists())? {
            same_device(source_dev, path_dev(candidate, deadline)?)?;
            return Ok(());
        }
    }
    bail!("cannot locate existing ancestor for {}", staging.display())
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
        torcul_policy::plan(&record.facts, &record.meta)? == record.plan,
        "saved plan is inconsistent"
    );
    Ok((output, sidecar, record))
}

pub fn verify(src: &Path, staging: &Path) -> Result<()> {
    let (output, sidecar, mut record) = load(src, staging)?;
    let previous = record.verified_staging.take();
    record.verification = None;
    files::write_json(&sidecar, &record)?;
    let (report, verified) =
        match verified_report(src, staging, &output, &record, previous.as_ref()) {
            Ok(value) => value,
            Err(error) => (VerifyReport::failed(format!("{error:#}")), None),
        };
    let pass = report.pass;
    let failures = report.failures.clone();
    record.verification = Some(report);
    record.verified_staging = verified;
    files::write_json(&sidecar, &record)?;
    if !pass {
        bail!("staging verification failed: {failures:?}");
    }
    Ok(())
}

fn verified_report(
    src: &Path,
    staging: &Path,
    output: &Path,
    record: &Record,
    previous: Option<&Fingerprint>,
) -> Result<(VerifyReport, Option<Fingerprint>)> {
    let deadline = Deadline::new();
    same_device(record.fingerprint.dev, path_dev(staging, deadline)?)?;
    ensure!(
        fingerprint(src)? == record.fingerprint,
        "source changed since remux"
    );
    let before = fingerprint(output)?;
    if let Some(previous) = previous {
        ensure!(&before == previous, "staging changed after verify");
    }
    let report = torcul_media::verify(src, &record.plan, output)?;
    ensure!(
        fingerprint(src)? == record.fingerprint,
        "source changed during verify"
    );
    ensure!(
        fingerprint(output)? == before,
        "staging changed during verify"
    );
    let verified = report.pass.then_some(before);
    Ok((report, verified))
}
