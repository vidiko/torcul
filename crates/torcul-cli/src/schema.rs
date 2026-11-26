// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{files, report::FileReport, staging::Record};
use anyhow::{Context, Result};
use schemars::{JsonSchema, generate::SchemaSettings};
use std::{fs, path::Path};
use torcul_job::{Job, JobRequest};
use torcul_media::io::Deadline;

pub fn write_all(out: &Path) -> Result<()> {
    Deadline::new()
        .run(|| fs::create_dir_all(out))
        .with_context(|| format!("create {}", out.display()))?;
    write_one::<JobRequest>(out, "job-request-1.schema.json", "torcul/job-request/1")?;
    write_one::<Job>(out, "job-1.schema.json", "torcul/job/1")?;
    write_one::<Record>(
        out,
        "staging-record-1.schema.json",
        "torcul/staging-record/1",
    )?;
    write_one::<Vec<FileReport>>(out, "plan-report.schema.json", "torcul/plan-report")?;
    Ok(())
}

fn write_one<T: JsonSchema>(out: &Path, name: &str, title: &str) -> Result<()> {
    let mut schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>();
    schema.insert("title".to_owned(), title.into());
    let mut bytes = serde_json::to_vec_pretty(&schema)?;
    bytes.push(b'\n');
    files::write_atomic(&out.join(name), &bytes)
}
