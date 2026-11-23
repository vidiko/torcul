// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::files;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use torcul_policy::{Plan, TitleMeta};

#[derive(Deserialize)]
struct Manifest {
    fixtures: Vec<MetaRow>,
}
#[derive(Deserialize)]
struct MetaRow {
    name: String,
    #[serde(flatten)]
    meta: TitleMeta,
}
pub type Metadata = BTreeMap<String, TitleMeta>;
pub fn metadata(path: &Path) -> Result<Metadata> {
    let manifest: Manifest = files::read_json(path)?;
    let mut map = Metadata::new();
    for row in manifest.fixtures {
        if map.insert(row.name.clone(), row.meta).is_some() {
            bail!("duplicate metadata name: {}", row.name);
        }
    }
    Ok(map)
}
pub fn title<'a>(path: &Path, meta: &'a Metadata) -> Result<&'a TitleMeta> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("file has no UTF-8 basename")?;
    meta.get(name)
        .with_context(|| format!("no metadata entry for {name}"))
}
#[derive(Serialize)]
pub struct FileReport {
    pub path: PathBuf,
    pub plan: Option<Plan>,
    pub error: Option<String>,
}
pub fn plans(paths: &[PathBuf], meta: &Metadata) -> Vec<FileReport> {
    paths
        .iter()
        .map(|path| {
            let result = title(path, meta)
                .and_then(|m| Ok(torcul_policy::plan(&torcul_media::probe(path)?, m)));
            match result {
                Ok(plan) => FileReport {
                    path: path.clone(),
                    plan: Some(plan),
                    error: None,
                },
                Err(error) => FileReport {
                    path: path.clone(),
                    plan: None,
                    error: Some(format!("{error:#}")),
                },
            }
        })
        .collect()
}
pub fn table(reports: &[FileReport]) {
    println!("FILE | KEEP (input indices) | REMUX | WARNINGS");
    for row in reports {
        let name = row.path.file_name().unwrap_or_default().to_string_lossy();
        if let Some(plan) = &row.plan {
            println!(
                "{} | {} | {} | {}",
                name,
                plan.kept_indices()
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
                if plan.needs_remux { "yes" } else { "no" },
                plan.warnings.join(",")
            );
        } else {
            println!(
                "{} | ERROR | - | {}",
                name,
                row.error.as_deref().unwrap_or("unknown error")
            );
        }
    }
}
