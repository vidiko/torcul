// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{files, report::FileReport, staging::Record};
use anyhow::{Context, Result};
use schemars::{
    JsonSchema, Schema,
    generate::SchemaSettings,
    transform::{RecursiveTransform, RestrictFormats, Transform},
};
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

/// schemars bounds `u8` and `u16` only; this adds the exact upper bound of `u32`, `u64` and
/// `usize` (`uint`, matching the 64-bit generator platform) before non-standard integer `format`
/// annotations are stripped.
fn integer_maximum(schema: &mut Schema) {
    let maximum: Value = match schema.get("format").and_then(Value::as_str) {
        Some("uint32") => u32::MAX.into(),
        Some("uint64") => u64::MAX.into(),
        Some("uint") => usize::MAX.into(),
        _ => return,
    };
    schema.insert("maximum".to_owned(), maximum);
}

fn generate<T: JsonSchema>(title: &str) -> Schema {
    let mut schema = SchemaSettings::draft2020_12()
        .with_transform(RecursiveTransform(integer_maximum))
        .into_generator()
        .into_root_schema_for::<T>();
    RestrictFormats::default().transform(&mut schema);
    schema.insert("title".to_owned(), title.into());
    schema
}

fn write_one<T: JsonSchema>(out: &Path, name: &str, title: &str) -> Result<()> {
    let schema = generate::<T>(title);
    let mut bytes = serde_json::to_vec_pretty(&schema)?;
    bytes.push(b'\n');
    files::write_atomic(&out.join(name), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use serde_json::json;

    #[derive(Serialize, JsonSchema)]
    struct UnsignedSample {
        byte: u8,
        short: u16,
        word: u32,
        wide: u64,
        index: usize,
    }

    #[test]
    fn generate_sets_unsigned_maxima_and_strips_custom_integer_formats() {
        let sample = UnsignedSample {
            byte: u8::MAX,
            short: u16::MAX,
            word: u32::MAX,
            wide: u64::MAX,
            index: usize::MAX,
        };
        let root = serde_json::to_value(generate::<UnsignedSample>("test/unsigned")).unwrap();
        let validator = jsonschema::draft202012::options()
            .should_validate_formats(true)
            .should_ignore_unknown_formats(false)
            .offline()
            .build(&root)
            .unwrap();
        assert!(validator.is_valid(&serde_json::to_value(&sample).unwrap()));

        let props = root["properties"].as_object().unwrap();
        let expected = [
            ("byte", json!(u8::MAX)),
            ("short", json!(u16::MAX)),
            ("word", json!(u32::MAX)),
            ("wide", json!(u64::MAX)),
            ("index", json!(usize::MAX)),
        ];
        for (field, max) in expected {
            let prop = props[field].as_object().unwrap();
            assert_eq!(prop.get("minimum"), Some(&json!(0)), "{field}");
            assert_eq!(prop.get("maximum"), Some(&max), "{field}");
            assert!(!prop.contains_key("format"), "{field} kept format");
        }
    }
}
