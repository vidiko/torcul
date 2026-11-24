// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "torcul",
    version,
    about = "Plan, stage and verify lossless media track selection"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Dry run. Reads media and metadata; writes only an optional JSON report.
    Plan {
        #[arg(long)]
        meta: PathBuf,
        #[arg(long)]
        json: Option<PathBuf>,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Stream copy selected tracks into staging. Sources remain untouched.
    Remux {
        #[arg(long)]
        meta: PathBuf,
        #[arg(long)]
        staging: PathBuf,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Compare staged tracks, metadata, chapters and packet payloads to source.
    Verify {
        #[arg(long)]
        staging: PathBuf,
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
}
