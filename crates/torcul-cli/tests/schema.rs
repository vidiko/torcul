// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use jsonschema::Validator;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use torcul_job::{Job, JobRequest};
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

fn validator_for(name: &str) -> Validator {
    let schema = read_json_file(&schemas_dir().join(name));
    jsonschema::draft202012::options()
        .should_validate_formats(true)
        .offline()
        .build(&schema)
        .unwrap_or_else(|e| panic!("build validator for {name}: {e}"))
}

fn run_schema_subcommand(out: &Path) {
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
    assert_eq!(job.history.len(), 4);
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

    let mut empty_langs = base;
    empty_langs["rule"]["track"]["languages"] = json!([]);
    assert!(!req_validator.is_valid(&empty_langs));
}

#[test]
fn job_id_timestamp_and_fingerprint_boundaries() {
    let job_validator = validator_for("job-1.schema.json");
    let base = read_json_file(&schemas_dir().join("examples/job-1.json"));

    let mut valid_32 = base.clone();
    valid_32["id"] = json!("0123456789abcdef0123456789abcdef");
    assert!(job_validator.is_valid(&valid_32));

    for bad_id in [
        "0123456789abcdef0123456789abcde",
        "0123456789abcdef0123456789abcdef0",
        "A123456789abcdef0123456789abcdef",
    ] {
        let mut doc = base.clone();
        doc["id"] = json!(bad_id);
        assert!(!job_validator.is_valid(&doc), "expected bad id: {bad_id}");
    }

    let mut bad_time = base.clone();
    bad_time["history"][0]["at"] = json!("yesterday");
    assert!(!job_validator.is_valid(&bad_time));

    let mut bad_fp = base;
    bad_fp["source_fingerprint"]["sha256"] = json!("sha256:short");
    assert!(!job_validator.is_valid(&bad_fp));
}
