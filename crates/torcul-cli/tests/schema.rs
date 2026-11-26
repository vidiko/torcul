// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use jsonschema::Validator;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::Duration,
};
use torcul_job::{Job, JobRequest, JobState};
use torcul_media::io::Deadline;
use wait_timeout::ChildExt;

const MAX_SCHEMA_ENTRIES: usize = 64;

fn schemas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
}

fn read_json_file(path: &Path) -> Value {
    let bytes = Deadline::new().run(|| fs::read(path)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn validator_with_formats(name: &str, formats: bool) -> Validator {
    let schema = read_json_file(&schemas_dir().join(name));
    jsonschema::draft202012::options()
        .should_validate_formats(formats)
        .offline()
        .build(&schema)
        .unwrap_or_else(|e| panic!("build validator for {name}: {e}"))
}

fn validator_for(name: &str) -> Validator {
    validator_with_formats(name, true)
}

fn invoke_schema_subcommand(out: &Path) -> (ExitStatus, String) {
    let deadline = Deadline::new();
    let logs = deadline.run(tempfile::tempdir).unwrap();
    let stderr_path = logs.path().join("stderr");
    let stderr = deadline.run(|| fs::File::create(&stderr_path)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_torcul"))
        .args(["schema", "--out"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .unwrap();
    let status = match child.wait_timeout(Duration::from_secs(60)).unwrap() {
        Some(status) => status,
        None => {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("torcul schema timed out");
        }
    };
    let err = deadline.run(|| fs::read_to_string(&stderr_path)).unwrap();
    (status, err)
}

fn run_schema_subcommand(out: &Path) {
    let (status, err) = invoke_schema_subcommand(out);
    assert!(status.success(), "torcul schema failed: {err}");
}

fn collect_schema_files(dir: &Path, allow_examples_dir: bool) -> BTreeMap<String, Vec<u8>> {
    let deadline = Deadline::new();
    let mut entries = deadline.run(|| fs::read_dir(dir)).unwrap();
    let mut files = BTreeMap::new();
    for step in 0..=MAX_SCHEMA_ENTRIES {
        let Some(entry) = deadline.run(|| entries.next().transpose()).unwrap() else {
            break;
        };
        assert!(step < MAX_SCHEMA_ENTRIES, "schema directory bound exceeded");
        let name = entry.file_name().into_string().unwrap();
        let kind = deadline.run(|| entry.file_type()).unwrap();
        if allow_examples_dir && kind.is_dir() && name == "examples" {
            continue;
        }
        assert!(kind.is_file(), "unexpected non-file entry {name}");
        assert!(
            name.ends_with(".schema.json"),
            "unexpected file in schemas dir: {name}"
        );
        let bytes = deadline.run(|| fs::read(entry.path())).unwrap();
        files.insert(name, bytes);
    }
    files
}

fn sample_staging_record() -> Value {
    let job = read_json_file(&schemas_dir().join("examples/job-1.json"));
    json!({
        "version": 1,
        "source": job["request"]["source"],
        "fingerprint": job["source_fingerprint"],
        "facts": job["facts"],
        "meta": job["request"]["title"],
        "plan": job["plan"],
        "verification": job["verdict"],
        "verified_staging": job["staged_fingerprint"],
    })
}

#[test]
fn generated_schemas_match_checked_in_files() {
    let temp = Deadline::new().run(tempfile::tempdir).unwrap();
    let nested_out = temp.path().join("generated");
    run_schema_subcommand(&nested_out);

    let generated = collect_schema_files(&nested_out, false);
    let checked_in = collect_schema_files(&schemas_dir(), true);
    assert_eq!(generated.len(), 4);
    assert_eq!(
        generated.keys().collect::<Vec<_>>(),
        checked_in.keys().collect::<Vec<_>>()
    );
    for (name, gen_bytes) in &generated {
        assert!(
            gen_bytes.ends_with(b"\n") && !gen_bytes.ends_with(b"\n\n"),
            "{name} must end with exactly one trailing newline"
        );
        let repo_bytes = &checked_in[name];
        assert_eq!(
            gen_bytes, repo_bytes,
            "checked-in schemas/{name} drifted from `torcul schema --out schemas`"
        );
    }
}

#[test]
fn schemas_validate_against_draft_2020_12_meta_schema() {
    let expected = [
        ("job-request-1.schema.json", "torcul/job-request/1"),
        ("job-1.schema.json", "torcul/job/1"),
        ("staging-record-1.schema.json", "torcul/staging-record/1"),
        ("plan-report.schema.json", "torcul/plan-report"),
    ];
    let checked_in = collect_schema_files(&schemas_dir(), true);
    assert_eq!(checked_in.len(), expected.len());

    for (name, title) in expected {
        let schema = read_json_file(&schemas_dir().join(name));
        assert_eq!(
            schema["$schema"], "https://json-schema.org/draft/2020-12/schema",
            "{name}"
        );
        assert_eq!(schema["title"], title, "{name}");
        jsonschema::draft202012::meta::validate(&schema)
            .unwrap_or_else(|e| panic!("{name} failed 2020-12 meta-schema: {e}"));
    }

    let mut invalid_schema = read_json_file(&schemas_dir().join("job-request-1.schema.json"));
    invalid_schema["type"] = json!("not-a-valid-type");
    assert!(jsonschema::draft202012::meta::validate(&invalid_schema).is_err());
}

#[test]
fn example_documents_validate_and_round_trip() {
    let req_val = read_json_file(&schemas_dir().join("examples/job-request-1.json"));
    let req_validator = validator_for("job-request-1.schema.json");
    assert!(
        req_validator.is_valid(&req_val),
        "{}",
        req_validator.iter_errors(&req_val).into_errors()
    );
    let req: JobRequest = serde_json::from_value(req_val.clone()).unwrap();
    assert_eq!(serde_json::to_value(&req).unwrap(), req_val);

    let job_val = read_json_file(&schemas_dir().join("examples/job-1.json"));
    let job_validator = validator_for("job-1.schema.json");
    assert!(
        job_validator.is_valid(&job_val),
        "{}",
        job_validator.iter_errors(&job_val).into_errors()
    );
    let job: Job = serde_json::from_value(job_val.clone()).unwrap();
    let states: Vec<JobState> = job.history.iter().map(|t| t.state).collect();
    assert_eq!(
        states,
        [
            JobState::Planned,
            JobState::Staged,
            JobState::Verified,
            JobState::Applying,
            JobState::Applied,
        ]
    );
    assert_eq!(job.state, JobState::Applied);
    assert_eq!(serde_json::to_value(&job).unwrap(), job_val);
}

#[test]
fn missing_required_fields_are_refused() {
    let req_validator = validator_for("job-request-1.schema.json");
    let mut req = read_json_file(&schemas_dir().join("examples/job-request-1.json"));
    req.as_object_mut().unwrap().remove("source");
    assert!(!req_validator.is_valid(&req));
    assert!(serde_json::from_value::<JobRequest>(req).is_err());

    let job_validator = validator_for("job-1.schema.json");
    let mut job = read_json_file(&schemas_dir().join("examples/job-1.json"));
    job.as_object_mut().unwrap().remove("state");
    assert!(!job_validator.is_valid(&job));
    assert!(serde_json::from_value::<Job>(job).is_err());

    let plan_validator = validator_for("plan-report.schema.json");
    let job_example = read_json_file(&schemas_dir().join("examples/job-1.json"));
    let valid_report = json!([{
        "path": "/srv/media/movies/Example.mkv",
        "plan": job_example["plan"],
        "error": null,
    }]);
    assert!(plan_validator.is_valid(&valid_report));
    let missing_path = json!([{ "plan": job_example["plan"], "error": null }]);
    assert!(!plan_validator.is_valid(&missing_path));

    let record_validator = validator_for("staging-record-1.schema.json");
    let mut record = sample_staging_record();
    assert!(record_validator.is_valid(&record));
    record.as_object_mut().unwrap().remove("plan");
    assert!(!record_validator.is_valid(&record));
}

#[test]
fn unknown_schema_and_sidecar_versions_are_refused() {
    let req_validator = validator_for("job-request-1.schema.json");
    let mut req = read_json_file(&schemas_dir().join("examples/job-request-1.json"));
    req["schema"] = json!("torcul/job-request/2");
    assert!(!req_validator.is_valid(&req));
    assert!(serde_json::from_value::<JobRequest>(req).is_err());

    let job_validator = validator_for("job-1.schema.json");
    let mut job = read_json_file(&schemas_dir().join("examples/job-1.json"));
    job["schema"] = json!("torcul/job/2");
    assert!(!job_validator.is_valid(&job));
    assert!(serde_json::from_value::<Job>(job).is_err());

    let record_validator = validator_for("staging-record-1.schema.json");
    let mut record = sample_staging_record();
    record["version"] = json!(2);
    assert!(!record_validator.is_valid(&record));
}

#[test]
fn request_boundaries_and_unknown_fields() {
    let req_validator = validator_for("job-request-1.schema.json");
    let base = read_json_file(&schemas_dir().join("examples/job-request-1.json"));

    for valid_days in [0_u16, 65535_u16] {
        let mut doc = base.clone();
        doc["rollback_days"] = json!(valid_days);
        assert!(req_validator.is_valid(&doc), "expected valid: {valid_days}");
        let parsed: JobRequest = serde_json::from_value(doc).unwrap();
        assert_eq!(parsed.rollback_days, valid_days);
    }

    for invalid_days in [json!(65536), json!(-1)] {
        let mut doc = base.clone();
        doc["rollback_days"] = invalid_days.clone();
        assert!(
            !req_validator.is_valid(&doc),
            "expected refused: {invalid_days}"
        );
        assert!(serde_json::from_value::<JobRequest>(doc).is_err());
    }

    let mut omitted = base.clone();
    omitted.as_object_mut().unwrap().remove("rollback_days");
    assert!(req_validator.is_valid(&omitted));
    let defaulted: JobRequest = serde_json::from_value(omitted).unwrap();
    assert_eq!(defaulted.rollback_days, 7);

    let mut unknown_top = base.clone();
    unknown_top["extra_field"] = json!(true);
    assert!(!req_validator.is_valid(&unknown_top));
    assert!(serde_json::from_value::<JobRequest>(unknown_top).is_err());
}

#[test]
fn request_title_and_languages_boundaries() {
    let req_validator = validator_for("job-request-1.schema.json");
    let base = read_json_file(&schemas_dir().join("examples/job-request-1.json"));

    let mut one_lang = base.clone();
    one_lang["rule"]["track"]["languages"] = json!(["eng"]);
    assert!(req_validator.is_valid(&one_lang));
    let parsed: JobRequest = serde_json::from_value(one_lang).unwrap();
    assert_eq!(parsed.rule.track.languages.len(), 1);

    let mut empty_langs = base.clone();
    empty_langs["rule"]["track"]["languages"] = json!([]);
    assert!(!req_validator.is_valid(&empty_langs));
    let err = serde_json::from_value::<JobRequest>(empty_langs).unwrap_err();
    assert!(
        err.to_string().contains("at least one language code"),
        "{err}"
    );

    let mut dup_langs = base.clone();
    dup_langs["rule"]["track"]["languages"] = json!(["eng", "eng"]);
    assert!(!req_validator.is_valid(&dup_langs));
    let dup_err = serde_json::from_value::<JobRequest>(dup_langs).unwrap_err();
    assert!(
        dup_err.to_string().contains("duplicate item in a set"),
        "{dup_err}"
    );

    let mut title_only_anime = base.clone();
    title_only_anime["title"] = json!({ "anime": true });
    assert!(req_validator.is_valid(&title_only_anime));
    let parsed: JobRequest = serde_json::from_value(title_only_anime).unwrap();
    assert!(parsed.title.anime && parsed.title.original.is_none());

    let mut title_extra = base;
    title_extra["title"]["year"] = json!(1999);
    assert!(!req_validator.is_valid(&title_extra));
    assert!(serde_json::from_value::<JobRequest>(title_extra).is_err());
}

#[test]
fn plan_report_entries_hold_a_plan() {
    let validator = validator_for("plan-report.schema.json");
    let plan = read_json_file(&schemas_dir().join("examples/job-1.json"))["plan"].clone();
    let path = "/srv/media/movies/Example.mkv";
    let cases = [
        (json!({ "path": path, "plan": plan, "error": null }), true),
        (
            json!({ "path": path, "plan": null, "error": "probe failed" }),
            false,
        ),
        (json!({ "path": path, "plan": null, "error": null }), false),
        (
            json!({ "path": path, "plan": plan, "error": "probe failed" }),
            false,
        ),
        (json!({ "path": path, "plan": plan }), false),
    ];
    for (entry, valid) in cases {
        let report = json!([entry]);
        assert_eq!(validator.is_valid(&report), valid, "{report}");
    }
    assert!(validator.is_valid(&json!([])));
}

fn assert_job_validity(validator: &Validator, doc: &Value, expected_valid: bool) {
    assert_eq!(validator.is_valid(doc), expected_valid, "{doc}");
    assert_eq!(
        serde_json::from_value::<Job>(doc.clone()).is_ok(),
        expected_valid,
        "serde disagreed with schema for {doc}"
    );
}

#[test]
fn restart_states_require_fingerprints_and_rollback_path() {
    let validator = validator_for("job-1.schema.json");
    let base = read_json_file(&schemas_dir().join("examples/job-1.json"));
    let check = |state: &str, expected: bool, edit: &dyn Fn(&mut Value)| {
        let mut doc = base.clone();
        doc["state"] = json!(state);
        edit(&mut doc);
        assert_job_validity(&validator, &doc, expected);
    };
    let keep = |_: &mut Value| {};
    let null_source = |d: &mut Value| d["source_fingerprint"] = Value::Null;
    let null_applied = |d: &mut Value| d["applied_fingerprint"] = Value::Null;
    let drop_staged = |d: &mut Value| {
        d.as_object_mut().unwrap().remove("staged_fingerprint");
    };
    let set_rollback = |d: &mut Value| d["rollback_path"] = json!("/srv/media/movies/Example.mkv");

    check("verified", true, &keep);
    check("verified", false, &drop_staged);
    check("applying", true, &keep);
    check("applying", false, &null_source);
    check("applying", false, &drop_staged);
    check("applied", true, &keep);
    check("applied", false, &null_applied);
    check("applied", false, &null_source);
    check("applied", false, &drop_staged);
    check("rolling_back", false, &keep);
    check("rolling_back", true, &set_rollback);
    check("rolling_back", false, &|d| {
        set_rollback(d);
        null_source(d);
    });
    check("held", false, &keep);
    check("held", true, &|d| {
        d["held"] = json!({ "library": 42, "staging": null });
    });
    check("failed", false, &keep);
    check("failed", true, &|d| {
        null_source(d);
        drop_staged(d);
        d["failure"] = json!({ "step": "apply", "error": "exchange failed" });
    });
    check("planned", true, &|d| {
        null_source(d);
        drop_staged(d);
        null_applied(d);
    });
}

#[test]
fn unsigned_fields_carry_exact_maximum() {
    let job_validator = validator_for("job-1.schema.json");
    let record_validator = validator_for("staging-record-1.schema.json");
    let max = json!(u64::MAX);
    let over: Value = serde_json::from_str("18446744073709551616").unwrap();

    let mut job = read_json_file(&schemas_dir().join("examples/job-1.json"));
    job["source_fingerprint"]["size"] = max.clone();
    assert!(job_validator.is_valid(&job));
    let parsed: Job = serde_json::from_value(job.clone()).unwrap();
    assert_eq!(parsed.source_fingerprint.unwrap().size, u64::MAX);
    job["source_fingerprint"]["size"] = over.clone();
    assert!(!job_validator.is_valid(&job));
    assert!(serde_json::from_value::<Job>(job).is_err());

    let mut record = sample_staging_record();
    record["fingerprint"]["size"] = max;
    assert!(record_validator.is_valid(&record));
    record["fingerprint"]["size"] = over;
    assert!(!record_validator.is_valid(&record));
}

#[test]
fn job_id_timestamp_and_fingerprint_boundaries() {
    let job_validator = validator_for("job-1.schema.json");
    let base = read_json_file(&schemas_dir().join("examples/job-1.json"));

    let mut valid_32 = base.clone();
    valid_32["id"] = json!("0123456789abcdef0123456789abcdef");
    assert_job_validity(&job_validator, &valid_32, true);

    for bad_id in [
        "0123456789abcdef0123456789abcde",
        "0123456789abcdef0123456789abcdef0",
        "A123456789abcdef0123456789abcdef",
    ] {
        let mut doc = base.clone();
        doc["id"] = json!(bad_id);
        assert_job_validity(&job_validator, &doc, false);
    }

    let mut empty_history = base.clone();
    empty_history["history"] = json!([]);
    assert_job_validity(&job_validator, &empty_history, false);

    let shape_only = validator_with_formats("job-1.schema.json", false);
    let mut bad_time = base.clone();
    bad_time["history"][0]["at"] = json!("yesterday");
    assert!(!shape_only.is_valid(&bad_time));
    assert_job_validity(&job_validator, &bad_time, false);

    let mut month_13 = base.clone();
    month_13["history"][0]["at"] = json!("2026-13-01T00:00:00Z");
    assert!(shape_only.is_valid(&month_13));
    assert!(serde_json::from_value::<Job>(month_13.clone()).is_ok());
    assert!(!job_validator.is_valid(&month_13));

    let mut fractional = base.clone();
    fractional["history"][0]["at"] = json!("2026-11-26T17:00:00.123Z");
    assert!(shape_only.is_valid(&fractional));
    assert_job_validity(&job_validator, &fractional, true);

    check_fingerprint_and_packet_boundaries(&job_validator, &base);
}

fn check_fingerprint_and_packet_boundaries(job_validator: &Validator, base: &Value) {
    let mut bad_fp = base.clone();
    bad_fp["source_fingerprint"]["sha256"] = json!("sha256:short");
    assert_job_validity(job_validator, &bad_fp, false);

    for bad_packet in [
        "short",
        "A123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ] {
        let mut doc = base.clone();
        doc["verdict"]["source_packets"][0]["sha256"] = json!(bad_packet);
        assert_job_validity(job_validator, &doc, false);
    }

    let mut max_mtime = base.clone();
    max_mtime["source_fingerprint"]["mtime_ns"] = json!(u64::MAX);
    assert_job_validity(job_validator, &max_mtime, true);

    let mut over_mtime = base.clone();
    over_mtime["source_fingerprint"]["mtime_ns"] =
        serde_json::from_str("18446744073709551616").unwrap();
    assert_job_validity(job_validator, &over_mtime, false);

    let mut dup_disp = base.clone();
    dup_disp["facts"]["tracks"][0]["dispositions"] = json!(["default", "default"]);
    assert_job_validity(job_validator, &dup_disp, false);
}

#[test]
fn schemas_build_with_strict_format_validation() {
    for name in [
        "job-request-1.schema.json",
        "job-1.schema.json",
        "staging-record-1.schema.json",
        "plan-report.schema.json",
    ] {
        let schema = read_json_file(&schemas_dir().join(name));
        jsonschema::draft202012::options()
            .should_validate_formats(true)
            .should_ignore_unknown_formats(false)
            .offline()
            .build(&schema)
            .unwrap_or_else(|e| panic!("{name} carries unknown format: {e}"));
    }

    let unknown_format = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "integer",
        "format": "uint64"
    });
    assert!(
        jsonschema::draft202012::options()
            .should_validate_formats(true)
            .should_ignore_unknown_formats(false)
            .offline()
            .build(&unknown_format)
            .is_err()
    );
}

#[test]
fn example_job_id_is_the_source_path_hash() {
    let job = read_json_file(&schemas_dir().join("examples/job-1.json"));
    let source = job["request"]["source"].as_str().unwrap();
    let hex: String = Sha256::digest(source.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(job["id"].as_str().unwrap(), &hex[..32]);
}

#[test]
fn schema_refuses_regular_file_destination() {
    let deadline = Deadline::new();
    let temp = deadline.run(tempfile::tempdir).unwrap();
    let file_out = temp.path().join("existing.txt");
    let original = b"keep-me-intact\n";
    deadline.run(|| fs::write(&file_out, original)).unwrap();

    let (status, err) = invoke_schema_subcommand(&file_out);
    assert!(!status.success(), "expected failure when --out is a file");
    assert!(!err.trim().is_empty(), "expected diagnostic on stderr");
    assert_eq!(deadline.run(|| fs::read(&file_out)).unwrap(), original);
}
