// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
//! Pure data contracts and JSON Schema definitions for `torcul/job-request/1` and `torcul/job/1`.
//!
//! These types model the trigger request written by an external scheduler (such as SnatchArr) and
//! the durable `job.json` state record owned by `torcul` (ADR-0002 decisions 9 to 15). This crate
//! performs no I/O and contains no execution runner.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};
use torcul_media::{VerifyReport, io::Fingerprint};
use torcul_policy::{FileFacts, Plan, TitleMeta};

const fn default_rollback_days() -> u16 {
    7
}

/// Schema identifier for a version 1 job request (`"torcul/job-request/1"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RequestSchema {
    #[serde(rename = "torcul/job-request/1")]
    V1,
}

/// Schema identifier for a version 1 job record (`"torcul/job/1"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum JobSchema {
    #[serde(rename = "torcul/job/1")]
    V1,
}

/// Execution depth requested for a job (`plan`, `stage` or `apply`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Plan,
    Stage,
    Apply,
}

/// Effective track-selection rule resolved by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackRule {
    /// ISO 639 language codes to keep.
    #[schemars(length(min = 1))]
    pub languages: BTreeSet<String>,
}

/// Effective processing rule resolved by the caller for this library item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Track-selection policy parameters.
    pub track: TrackRule,
}

/// Job request contract (`torcul/job-request/1`) written by a caller to trigger `torcul`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    /// Contract identifier (`"torcul/job-request/1"`).
    pub schema: RequestSchema,
    /// Path to the library media file.
    pub source: PathBuf,
    /// Execution stop point (`plan`, `stage` or `apply`).
    pub mode: Mode,
    /// Effective rule resolved by the caller.
    pub rule: Rule,
    /// Title facts needed by the policy planner (original language and anime flag).
    pub title: TitleMeta,
    /// Staging root directory on the same filesystem device as `source`.
    pub staging_root: PathBuf,
    /// Days to retain the swapped-out original as a rollback copy; defaults to 7.
    #[serde(default = "default_rollback_days")]
    pub rollback_days: u16,
}

/// Lifecycle state recorded in `job.json` (ADR-0002 decision 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Plan written, nothing staged.
    Planned,
    /// The plan changes nothing; final.
    Unchanged,
    /// Output complete in staging.
    Staged,
    /// Verdict passed; ready to apply.
    Verified,
    /// Verdict failed; final, library untouched.
    Rejected,
    /// Exchange in progress.
    Applying,
    /// The library holds the output; the original is the rollback copy.
    Applied,
    /// Reverse exchange in progress.
    RollingBack,
    /// The library holds the original again; final.
    RolledBack,
    /// A restart found the files in no state it can finish; both files, the record and the applied-index entry are kept until an operator commits the job.
    Held,
    /// Rollback copy deleted; final.
    Committed,
    /// A step failed; the record names the step and the error; final.
    Failed,
}

/// Pipeline execution step recorded on failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepName {
    Plan,
    Stage,
    Verify,
    Apply,
    Rollback,
    Commit,
    Restart,
}

/// Recorded state transition with timestamp and `torcul` version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Transition {
    /// State entered by this transition.
    pub state: JobState,
    /// RFC 3339 UTC timestamp when the state was recorded.
    #[schemars(extend("format" = "date-time"))]
    pub at: String,
    /// Version of `torcul` that wrote this transition.
    pub torcul_version: String,
}

/// Step name and diagnostic recorded when a job enters `failed`, a swap back during apply included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Failure {
    /// Execution step that failed.
    pub step: StepName,
    /// Human-readable error message.
    pub error: String,
}

/// Inodes observed at the library path and staging output when a restart enters `held`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HeldInodes {
    /// Inode found at the library path, or `None` if missing.
    pub library: Option<u64>,
    /// Inode found at the staging output path, or `None` if missing.
    pub staging: Option<u64>,
}

/// Durable job state record (`torcul/job/1`) written atomically to `<staging_root>/<id>/job.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Job {
    /// Contract identifier (`"torcul/job/1"`).
    pub schema: JobSchema,
    /// First 32 lowercase hex digits of the SHA-256 of the canonical source path.
    #[schemars(regex(pattern = r"^[0-9a-f]{32}$"))]
    pub id: String,
    /// Original request that created or replaced this job.
    pub request: JobRequest,
    /// Current lifecycle state of the job.
    pub state: JobState,
    /// Ordered log of state transitions.
    pub history: Vec<Transition>,
    /// Fingerprint of the library source file captured at plan time.
    pub source_fingerprint: Option<Fingerprint>,
    /// Container, chapter and stream facts probed from the source file.
    pub facts: Option<FileFacts>,
    /// Track-selection and stream plan derived from `facts`, `request.rule` and `request.title`.
    pub plan: Option<Plan>,
    /// Version of the policy planner that derived `plan`.
    pub policy_version: Option<String>,
    /// Verification report comparing the staged output against the source and plan.
    pub verdict: Option<VerifyReport>,
    /// Fingerprint of the verified output in staging prior to exchange.
    pub staged_fingerprint: Option<Fingerprint>,
    /// Fingerprint of the applied output at the library path after exchange.
    pub applied_fingerprint: Option<Fingerprint>,
    /// Library path recorded when a rollback begins.
    pub rollback_path: Option<PathBuf>,
    /// Inodes observed when restart recovery places the job into `held`.
    pub held: Option<HeldInodes>,
    /// Step and error message when an operation fails.
    pub failure: Option<Failure>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_enums_accept_only_v1_strings() {
        assert_eq!(
            serde_json::to_value(RequestSchema::V1).unwrap(),
            json!("torcul/job-request/1")
        );
        assert_eq!(
            serde_json::to_value(JobSchema::V1).unwrap(),
            json!("torcul/job/1")
        );
        assert!(serde_json::from_value::<RequestSchema>(json!("torcul/job-request/2")).is_err());
        assert!(serde_json::from_value::<JobSchema>(json!("torcul/job/2")).is_err());
    }

    #[test]
    fn job_request_defaults_rollback_days_and_denies_unknown_fields() {
        let raw = json!({
            "schema": "torcul/job-request/1",
            "source": "/media/film.mkv",
            "mode": "stage",
            "rule": { "track": { "languages": ["eng"] } },
            "title": { "original": "eng", "anime": false },
            "staging_root": "/media/.staging"
        });
        let parsed: JobRequest = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(parsed.rollback_days, 7);

        let mut extra = raw;
        extra["unexpected"] = json!(1);
        assert!(serde_json::from_value::<JobRequest>(extra).is_err());
    }

    #[test]
    fn state_and_step_enums_use_snake_case() {
        assert_eq!(
            serde_json::to_value(JobState::RollingBack).unwrap(),
            json!("rolling_back")
        );
        assert_eq!(
            serde_json::to_value(JobState::RolledBack).unwrap(),
            json!("rolled_back")
        );
        assert_eq!(
            serde_json::to_value(StepName::Restart).unwrap(),
            json!("restart")
        );
        assert_eq!(serde_json::to_value(Mode::Apply).unwrap(), json!("apply"));
    }
}
