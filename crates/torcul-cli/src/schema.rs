// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{files, report::FileReport, staging::Record};
use anyhow::{Context, Result};
use schemars::{JsonSchema, Schema, generate::SchemaSettings, transform::RecursiveTransform};
use serde_json::Value;
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

/// schemars bounds `u8` and `u16` only; this adds the exact upper bound of `u32` and `u64`.
/// `usize` (`uint`) depends on the writer's platform and `u128` exceeds a JSON number here, so
/// both keep their lower bound only.
fn integer_maximum(schema: &mut Schema) {
    let maximum: Value = match schema.get("format").and_then(Value::as_str) {
        Some("uint32") => u32::MAX.into(),
        Some("uint64") => u64::MAX.into(),
        _ => return,
    };
    schema.insert("maximum".to_owned(), maximum);
}

fn write_one<T: JsonSchema>(out: &Path, name: &str, title: &str) -> Result<()> {
    let mut schema = SchemaSettings::draft2020_12()
        .with_transform(RecursiveTransform(integer_maximum))
        .into_generator()
        .into_root_schema_for::<T>();
    schema.insert("title".to_owned(), title.into());
    let mut bytes = serde_json::to_vec_pretty(&schema)?;
    bytes.push(b'\n');
    files::write_atomic(&out.join(name), &bytes)
}
