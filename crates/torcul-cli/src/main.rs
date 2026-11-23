// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod apply;
mod args;
mod files;
mod report;
mod staging;
use anyhow::{Result, bail};
use clap::Parser;

fn main() {
    if let Err(error) = run(args::Cli::parse()) {
        eprintln!("torcul: {error:#}");
        std::process::exit(1);
    }
}
fn run(cli: args::Cli) -> Result<()> {
    match cli.command {
        args::Command::Plan {
            meta,
            json,
            files: paths,
        } => {
            let meta = report::metadata(&meta)?;
            let reports = report::plans(&files::expand(&paths)?, &meta);
            report::table(&reports);
            if let Some(path) = json {
                files::write_json(&path, &reports)?;
            }
            if reports.iter().any(|r| r.error.is_some()) {
                bail!("one or more files failed planning");
            }
            Ok(())
        }
        args::Command::Remux {
            meta,
            staging,
            files,
        } => {
            let meta = report::metadata(&meta)?;
            batch(&files, |file| staging::prepare(file, &staging, &meta))
        }
        args::Command::Verify { staging, files } => {
            batch(&files, |file| staging::verify(file, &staging))
        }
        args::Command::Apply { staging, files } => {
            batch(&files, |file| apply::apply(file, &staging))
        }
    }
}
fn batch(
    paths: &[std::path::PathBuf],
    operation: impl Fn(&std::path::Path) -> Result<()>,
) -> Result<()> {
    let mut failed = false;
    for path in paths {
        match operation(path) {
            Ok(()) => println!("{}: ok", path.display()),
            Err(e) => {
                eprintln!("{}: {e:#}", path.display());
                failed = true;
            }
        }
    }
    if failed {
        bail!("one or more files failed");
    }
    Ok(())
}
