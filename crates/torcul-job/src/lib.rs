// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
//! Pure data contracts and JSON Schema definitions for `torcul/job-request/1` and `torcul/job/1`.
//!
//! These types model the trigger request written by an external scheduler (such as SnatchArr) and
//! the durable `job.json` state record owned by `torcul` (ADR-0002 decisions 9 to 15). This crate
//! performs no I/O and contains no execution runner.

use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Map, Value, json};
use std::{collections::BTreeSet, path::PathBuf};
use torcul_media::{VerifyReport, io::Fingerprint, pattern};
use torcul_policy::{FileFacts, Plan, TitleMeta};

const fn default_rollback_days() -> u16 {
    7
}

/// Lowercase 32-hex-digit job identifier derived from the canonical source path SHA-256.
pub const JOB_ID: &str = r"^[0-9a-f]{32}$";

/// RFC 3339 UTC timestamp shape, checked without opt-in format validation.
const UTC_TIMESTAMP: &str = r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(\.[0-9]+)?Z$";

fn deserialize_job_id<'de, D: Deserializer<'de>>(input: D) -> Result<String, D::Error> {
    pattern::matching(String::deserialize(input)?, JOB_ID, "job id")
}

fn deserialize_utc_timestamp<'de, D: Deserializer<'de>>(input: D) -> Result<String, D::Error> {
    pattern::matching(String::deserialize(input)?, UTC_TIMESTAMP, "UTC timestamp")
}

fn non_empty_languages<'de, D: Deserializer<'de>>(input: D) -> Result<BTreeSet<String>, D::Error> {
    let languages = torcul_policy::unique_set::<_, String>(input)?;
    if languages.is_empty() {
        return Err(de::Error::invalid_length(0, &"at least one language code"));
    }
    Ok(languages)
}

fn non_empty_history<'de, D: Deserializer<'de>>(input: D) -> Result<Vec<Transition>, D::Error> {
    let history = Vec::<Transition>::deserialize(input)?;
    if history.is_empty() {
        return Err(de::Error::invalid_length(0, &"at least one transition"));
    }
    Ok(history)
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
    /// ISO 639 language codes to keep; at least one.
    #[serde(deserialize_with = "non_empty_languages")]
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

/// Title facts the caller resolves for the policy planner; unknown fields are refused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestTitle {
    /// ISO 639 code of the title's original language, if known.
    pub original: Option<String>,
    /// Whether the title is anime.
    pub anime: bool,
}

impl From<RequestTitle> for TitleMeta {
    fn from(title: RequestTitle) -> Self {
        Self {
            original: title.original,
            anime: title.anime,
        }
    }
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
    pub title: RequestTitle,
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
    #[schemars(regex(pattern = UTC_TIMESTAMP))]
    #[serde(deserialize_with = "deserialize_utc_timestamp")]
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

struct StateRule {
    states: &'static [&'static str],
    fields: &'static [&'static str],
}

/// State-dependent non-null field rules shared by the JSON Schema transform and `serde`; each
/// row names the ADR-0002 decision that needs the field in those states.
const STATE_RULES: &[StateRule] = &[
    // Decision 13: apply compares the staged file with the verdict's fingerprint; decision 12:
    // entry liveness reads it.
    StateRule {
        states: &["verified", "applying", "applied", "rolling_back", "held"],
        fields: &["staged_fingerprint"],
    },
    // Decision 15: restart inodes; decision 14: rollback step 1 checks the rollback copy.
    StateRule {
        states: &["applying", "applied", "rolling_back"],
        fields: &["source_fingerprint"],
    },
    // Decision 14: rollback steps 1 and 4.
    StateRule {
        states: &["applied", "rolling_back"],
        fields: &["applied_fingerprint"],
    },
    // Decision 14: step 2.
    StateRule {
        states: &["rolling_back"],
        fields: &["rollback_path"],
    },
    // Decision 15: records held with the inodes found.
    StateRule {
        states: &["held"],
        fields: &["held"],
    },
    // Decision 14: a refusal records failed.
    StateRule {
        states: &["failed"],
        fields: &["failure"],
    },
];

/// Requires non-null `fields` whenever the job's state is one of `states`.
fn require_when(states: &[&str], fields: &[&str]) -> Value {
    let not_null: Map<String, Value> = fields
        .iter()
        .map(|field| ((*field).to_owned(), json!({ "not": { "type": "null" } })))
        .collect();
    json!({
        "if": { "properties": { "state": { "enum": states } }, "required": ["state"] },
        "then": { "properties": not_null, "required": fields },
    })
}

fn require_state_fields(schema: &mut Schema) {
    let rules: Vec<Value> = STATE_RULES
        .iter()
        .map(|rule| require_when(rule.states, rule.fields))
        .collect();
    schema.insert("allOf".to_owned(), Value::Array(rules));
}

fn check_state_fields(doc: &Value) -> Result<(), String> {
    let Some(state) = doc.get("state").and_then(Value::as_str) else {
        return Ok(());
    };
    for rule in STATE_RULES {
        if rule.states.contains(&state) {
            for &field in rule.fields {
                if doc.get(field).is_none_or(Value::is_null) {
                    return Err(format!("job in state {state} must set {field}"));
                }
            }
        }
    }
    Ok(())
}

/// Durable job state record (`torcul/job/1`) written atomically to `<staging_root>/<id>/job.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[schemars(transform = require_state_fields)]
pub struct Job {
    /// Contract identifier (`"torcul/job/1"`).
    pub schema: JobSchema,
    /// First 32 lowercase hex digits of the SHA-256 of the canonical source path.
    #[schemars(regex(pattern = JOB_ID))]
    pub id: String,
    /// Original request that created or replaced this job.
    pub request: JobRequest,
    /// Current lifecycle state of the job.
    pub state: JobState,
    /// Ordered log of state transitions.
    #[schemars(length(min = 1))]
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

#[derive(Deserialize)]
struct JobFields {
    schema: JobSchema,
    #[serde(deserialize_with = "deserialize_job_id")]
    id: String,
    request: JobRequest,
    state: JobState,
    #[serde(deserialize_with = "non_empty_history")]
    history: Vec<Transition>,
    source_fingerprint: Option<Fingerprint>,
    facts: Option<FileFacts>,
    plan: Option<Plan>,
    policy_version: Option<String>,
    verdict: Option<VerifyReport>,
    staged_fingerprint: Option<Fingerprint>,
    applied_fingerprint: Option<Fingerprint>,
    rollback_path: Option<PathBuf>,
    held: Option<HeldInodes>,
    failure: Option<Failure>,
}

impl From<JobFields> for Job {
    fn from(fields: JobFields) -> Self {
        let JobFields {
            schema,
            id,
            request,
            state,
            history,
            source_fingerprint,
            facts,
            plan,
            policy_version,
            verdict,
            staged_fingerprint,
            applied_fingerprint,
            rollback_path,
            held,
            failure,
        } = fields;
        Self {
            schema,
            id,
            request,
            state,
            history,
            source_fingerprint,
            facts,
            plan,
            policy_version,
            verdict,
            staged_fingerprint,
            applied_fingerprint,
            rollback_path,
            held,
            failure,
        }
    }
}

impl<'de> Deserialize<'de> for Job {
    fn deserialize<D: Deserializer<'de>>(input: D) -> Result<Self, D::Error> {
        let doc = Value::deserialize(input)?;
        check_state_fields(&doc).map_err(de::Error::custom)?;
        serde_json::from_value::<JobFields>(doc)
            .map(Self::from)
            .map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_fingerprint() -> Value {
        json!({
            "dev": 1,
            "ino": 2,
            "size": 100,
            "mtime_ns": 500,
            "sha256": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        })
    }

    fn sample_job_all_fields() -> Value {
        let fp = sample_fingerprint();
        json!({
            "schema": "torcul/job/1",
            "id": "efd9ffebd9b273b7bf9cb4d8758ad451",
            "request": {
                "schema": "torcul/job-request/1",
                "source": "/media/film.mkv",
                "mode": "apply",
                "rule": { "track": { "languages": ["eng"] } },
                "title": { "original": "eng", "anime": false },
                "staging_root": "/media/.staging",
                "rollback_days": 7,
            },
            "state": "planned",
            "history": [{
                "state": "planned",
                "at": "2026-11-26T17:00:00Z",
                "torcul_version": "0.1.0",
            }],
            "source_fingerprint": fp,
            "facts": null,
            "plan": null,
            "policy_version": null,
            "verdict": null,
            "staged_fingerprint": fp,
            "applied_fingerprint": fp,
            "rollback_path": "/media/film.mkv",
            "held": { "library": 2, "staging": null },
            "failure": { "step": "apply", "error": "exchange failed" },
        })
    }

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
    fn request_title_converts_to_planner_metadata() {
        let title = RequestTitle {
            original: Some("jpn".to_owned()),
            anime: true,
        };
        let meta = TitleMeta::from(title);
        assert_eq!(meta.original.as_deref(), Some("jpn"));
        assert!(meta.anime);
        assert_eq!(
            TitleMeta::from(RequestTitle::default()),
            TitleMeta::default()
        );
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

    #[test]
    fn duplicate_languages_are_refused_by_serde() {
        let dup = json!({ "languages": ["eng", "eng"] });
        let err = serde_json::from_value::<TrackRule>(dup).unwrap_err();
        assert!(
            err.to_string().contains("duplicate item in a set"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn state_rules_reference_valid_states_and_job_properties() {
        let root = serde_json::to_value(schemars::schema_for!(Job)).unwrap();
        let props = root["properties"].as_object().unwrap();
        assert_eq!(STATE_RULES.len(), 6);
        for rule in STATE_RULES {
            assert!(!rule.states.is_empty());
            assert!(!rule.fields.is_empty());
            for &state in rule.states {
                serde_json::from_value::<JobState>(json!(state))
                    .unwrap_or_else(|e| panic!("invalid JobState {state}: {e}"));
            }
            for &field in rule.fields {
                assert!(
                    props.contains_key(field),
                    "STATE_RULES field {field} missing from Job schema properties"
                );
            }
        }
    }

    #[test]
    fn state_rules_accept_and_refuse_each_row_via_serde() {
        let base = sample_job_all_fields();
        for rule in STATE_RULES {
            for &state in rule.states {
                let mut passing = base.clone();
                passing["state"] = json!(state);
                serde_json::from_value::<Job>(passing.clone())
                    .unwrap_or_else(|e| panic!("expected {state} with fields set to pass: {e}"));

                for &field in rule.fields {
                    let mut failing = passing.clone();
                    failing[field] = Value::Null;
                    let err = serde_json::from_value::<Job>(failing).unwrap_err();
                    assert!(
                        err.to_string()
                            .contains(&format!("job in state {state} must set {field}")),
                        "unexpected error for ({state}, {field}): {err}"
                    );
                }
            }
        }
    }

    #[test]
    fn job_serde_checks_id_timestamp_and_non_empty_history() {
        let base = sample_job_all_fields();

        let mut bad_id = base.clone();
        bad_id["id"] = json!("A123456789abcdef0123456789abcdef");
        assert!(serde_json::from_value::<Job>(bad_id).is_err());

        let mut bad_at = base.clone();
        bad_at["history"][0]["at"] = json!("yesterday");
        assert!(serde_json::from_value::<Job>(bad_at).is_err());

        let mut month_13 = base.clone();
        month_13["history"][0]["at"] = json!("2026-13-01T00:00:00Z");
        assert!(serde_json::from_value::<Job>(month_13).is_ok());

        let mut empty_hist = base;
        empty_hist["history"] = json!([]);
        let err = serde_json::from_value::<Job>(empty_hist).unwrap_err();
        assert!(err.to_string().contains("at least one transition"), "{err}");
    }
}
