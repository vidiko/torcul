// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod args;
mod files;
mod report;
mod schema;
mod staging;
use anyhow::{Result, bail};
use clap::Parser;
use std::io::Write;
use torcul_media::io::Deadline;

fn main() {
    if let Err(error) = run(args::Cli::parse()) {
        let _ = writeln!(std::io::stderr(), "torcul: {error:#}");
        std::process::exit(1);
    }
}

fn run(cli: args::Cli) -> Result<()> {
    match cli.command {
        args::Command::Plan {
            meta: meta_path,
            json,
            files: paths,
        } => {
            let expanded = files::expand(&paths)?;
            if let Some(path) = &json {
                files::check_json_destination(path, &meta_path, &expanded)?;
            }
            let meta = report::metadata(&meta_path)?;
            let reports = report::plans(&expanded, &meta);
            report::table(&reports)?;
            if reports.iter().any(|r| r.error.is_some()) {
                bail!("one or more files failed planning");
            }
            if let Some(path) = json {
                files::write_json(&path, &reports)?;
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
        args::Command::Schema { out } => schema::write_all(&out),
    }
}

fn batch(
    paths: &[std::path::PathBuf],
    operation: impl Fn(&std::path::Path) -> Result<()>,
) -> Result<()> {
    let mut failed = false;
    for path in paths {
        match operation(path) {
            Ok(()) => write_line(std::io::stdout().lock(), &format!("{}: ok", path.display()))?,
            Err(e) => {
                write_line(
                    std::io::stderr().lock(),
                    &format!("{}: {e:#}", path.display()),
                )?;
                failed = true;
            }
        }
    }
    if failed {
        bail!("one or more files failed");
    }
    Ok(())
}

fn write_line(mut stream: impl Write, line: &str) -> Result<()> {
    let deadline = Deadline::new();
    deadline.run(|| {
        writeln!(stream, "{line}")?;
        stream.flush()
    })?;
    Ok(())
}
