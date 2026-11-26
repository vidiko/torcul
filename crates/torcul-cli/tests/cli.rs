// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
#[path = "../../torcul-media/tests/support/mod.rs"]
mod support;

use std::{
    ffi::OsStr,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use support::*;
use tempfile::TempDir;
use torcul_media::io::{Deadline, Fingerprint, fingerprint};

fn assert_matches_schema(schema_file: &str, instance: &serde_json::Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas")
        .join(schema_file);
    let bytes = Deadline::new().run(|| fs::read(&path)).unwrap();
    let schema: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .offline()
        .build(&schema)
        .unwrap();
    assert!(
        validator.is_valid(instance),
        "{schema_file}: {}",
        validator.iter_errors(instance).into_errors()
    );
}

fn cli(args: &[&OsStr]) -> Output {
    execute(
        Command::new(env!("CARGO_BIN_EXE_torcul")).args(args),
        Duration::from_secs(600),
    )
}

fn command(name: &str, meta: Option<&Path>, staging: Option<&Path>, file: &Path) -> Output {
    let mut args = vec![OsStr::new(name)];
    if let Some(meta) = meta {
        args.extend([OsStr::new("--meta"), meta.as_os_str()]);
    }
    if let Some(staging) = staging {
        args.extend([OsStr::new("--staging"), staging.as_os_str()]);
    }
    args.push(file.as_os_str());
    cli(&args)
}

fn prepared(name: &str) -> (TempDir, PathBuf, PathBuf) {
    let (temp, copy) = copy_fixture(name);
    let staging = temp.path().join("staging");
    let meta = fixtures().dir.path().join("manifest.json");
    let out = command("remux", Some(&meta), Some(&staging), &copy);
    assert!(out.success, "{}", out.stderr);
    (temp, copy, staging)
}

#[test]
fn cli_apply_subcommand_removed() {
    let (_tmp, src, staging) = prepared("f07_default_dropped.mkv");
    let before = fs::read(&src).unwrap();
    let verify_out = command("verify", None, Some(&staging), &src);
    assert!(verify_out.success, "{}", verify_out.stderr);

    let out = command("apply", None, Some(&staging), &src);
    assert!(!out.success);
    assert!(
        out.stderr.contains("unrecognized subcommand 'apply'"),
        "{}",
        out.stderr
    );
    assert_eq!(fs::read(&src).unwrap(), before);

    let help = cli(&[OsStr::new("--help")]);
    assert!(help.success);
    assert!(!help.stdout.contains("\n  apply"), "{}", help.stdout);
}

#[test]
fn cli_dry_run_report_matches_oracle_and_marks_pending_lid() {
    let fx = fixtures();
    let tmp = tempfile::tempdir().unwrap();
    let json = tmp.path().join("plan.json");
    let meta = fx.dir.path().join("manifest.json");
    let out = cli(&[
        OsStr::new("plan"),
        OsStr::new("--meta"),
        meta.as_os_str(),
        OsStr::new("--json"),
        json.as_os_str(),
        fx.dir.path().as_os_str(),
    ]);
    assert!(out.success, "{}", out.stderr);
    println!("{}", out.stdout);
    assert!(
        out.stdout.contains(
            "f01_movie_en_de_extra.mkv | 0,1,2,5,6,7,8,9,12 | drop (pending LID): 3,4,10,11 | yes |"
        ),
        "{}",
        out.stdout
    );
    let report_doc: serde_json::Value = serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_matches_schema("plan-report.schema.json", &report_doc);
    let rows = report_doc.as_array().unwrap();
    assert_eq!(rows.len(), 11);
    for (report, oracle) in rows.iter().zip(&fx.manifest.fixtures) {
        let p: torcul_policy::Plan = serde_json::from_value(report["plan"].clone()).unwrap();
        assert_eq!(p.kept_indices(), oracle.expect);
        assert_eq!(p.warnings, oracle.warnings);
        assert_eq!(p.needs_remux, oracle.needs_remux);
        let decisions = report["plan"]["decisions"].as_array().unwrap();
        for decision in decisions {
            let index = decision["index"].as_u64().unwrap() as usize;
            if oracle.expect.contains(&index) {
                assert!(
                    decision.get("lid").is_none(),
                    "{} kept stream {index} must omit lid: {decision}",
                    oracle.name
                );
            } else {
                assert_eq!(
                    decision.get("lid").and_then(|v| v.as_str()),
                    Some("pending"),
                    "{} dropped stream {index} must mark lid pending: {decision}",
                    oracle.name
                );
            }
        }
    }
}

#[test]
fn cli_missing_metadata_and_empty_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let meta = tmp.path().join("meta.json");
    fs::write(&meta, br#"{"fixtures":[]}"#).unwrap();
    let empty = tempfile::tempdir().unwrap();
    let out = command("plan", Some(&meta), None, empty.path());
    assert!(out.success, "{}", out.stderr);
    let file = fixtures().dir.path().join("f01_movie_en_de_extra.mkv");
    let out = command("plan", Some(&meta), None, &file);
    assert!(!out.success);
    assert!(out.stdout.contains("no metadata entry"), "{}", out.stdout);
}

#[test]
fn cli_invalid_original_language_fails_planning() {
    for bad in ["xx", "english"] {
        let tmp = tempfile::tempdir().unwrap();
        let meta = tmp.path().join("meta.json");
        fs::write(
            &meta,
            format!(
                r#"{{"fixtures":[{{"name":"f01_movie_en_de_extra.mkv","original":"{bad}","anime":false}}]}}"#
            ),
        )
        .unwrap();
        let file = fixtures().dir.path().join("f01_movie_en_de_extra.mkv");
        let out = command("plan", Some(&meta), None, &file);
        assert!(!out.success, "expected failure for {bad}");
        assert!(
            out.stdout.contains("invalid ISO 639 original language"),
            "{}",
            out.stdout
        );
    }
}

#[test]
fn cli_clean_files_write_nothing() {
    for name in ["f06_already_clean.mkv", "f11_no_audio_matches_original.mkv"] {
        let (temp, src) = copy_fixture(name);
        let staging = temp.path().join("staging");
        let out = command(
            "remux",
            Some(&fixtures().dir.path().join("manifest.json")),
            Some(&staging),
            &src,
        );
        assert!(out.success, "{}", out.stderr);
        assert!(!staging.exists());
    }
}

#[test]
fn cli_remux_sidecar_preserves_pending_lid_and_staging_fingerprint() {
    let (_tmp, src, staging) = prepared("f01_movie_en_de_extra.mkv");
    let staged = staging.join("f01_movie_en_de_extra.mkv");
    let sidecar = staging.join("f01_movie_en_de_extra.mkv.torcul.json");

    let initial: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    assert_matches_schema("staging-record-1.schema.json", &initial);
    assert_eq!(initial["plan"]["decisions"][3]["lid"], "pending");
    assert!(initial["plan"]["decisions"][0].get("lid").is_none());
    assert!(initial["verified_staging"].is_null());

    let out = command("verify", None, Some(&staging), &src);
    assert!(out.success, "{}", out.stderr);

    let after_verify: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    assert_matches_schema("staging-record-1.schema.json", &after_verify);
    assert!(after_verify.get("verified_staging_sha256").is_none());
    let recorded: Fingerprint =
        serde_json::from_value(after_verify["verified_staging"].clone()).unwrap();
    assert_eq!(recorded, fingerprint(&staged).unwrap());

    // Re-verifying an unchanged staged file still matches and passes.
    let again = command("verify", None, Some(&staging), &src);
    assert!(again.success, "{}", again.stderr);

    // Touching staged file mtime after verify causes mismatch and fails verify.
    let bumped = fs::metadata(&staged).unwrap().modified().unwrap() + Duration::from_secs(5);
    fs::File::open(&staged)
        .unwrap()
        .set_modified(bumped)
        .unwrap();
    assert_ne!(recorded, fingerprint(&staged).unwrap());
    let failed = command("verify", None, Some(&staging), &src);
    assert!(!failed.success);
    let after_touch: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    assert_eq!(after_touch["verification"]["pass"], false);
    assert!(after_touch["verified_staging"].is_null());
}

#[test]
fn cli_verify_rejects_unsupported_sidecar_version() {
    let (_tmp, src, staging) = prepared("f07_default_dropped.mkv");
    let sidecar = staging.join("f07_default_dropped.mkv.torcul.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    record["version"] = serde_json::json!(2);
    fs::write(&sidecar, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let out = command("verify", None, Some(&staging), &src);
    assert!(!out.success);
    assert!(
        out.stderr.contains("unsupported sidecar version"),
        "{}",
        out.stderr
    );
}

#[test]
fn cli_verify_changed_source_records_failure() {
    let (_tmp, src, staging) = prepared("f07_default_dropped.mkv");
    let out = command("verify", None, Some(&staging), &src);
    assert!(out.success, "{}", out.stderr);
    fs::OpenOptions::new()
        .append(true)
        .open(&src)
        .unwrap()
        .write_all(b"x")
        .unwrap();
    let out = command("verify", None, Some(&staging), &src);
    assert!(!out.success);
    let record: serde_json::Value = serde_json::from_slice(
        &fs::read(staging.join("f07_default_dropped.mkv.torcul.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["verification"]["pass"], false);
}

#[test]
fn cli_verify_tampered_staging_records_failure() {
    let (_tmp, src, staging) = prepared("f08_mp4_en_fr.mp4");
    let staged = staging.join("f08_mp4_en_fr.mp4");
    let mut bytes = fs::read(&staged).unwrap();
    bytes[64] ^= 0x01;
    fs::write(&staged, bytes).unwrap();
    let out = command("verify", None, Some(&staging), &src);
    assert!(!out.success);
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(staging.join("f08_mp4_en_fr.mp4.torcul.json")).unwrap())
            .unwrap();
    assert_eq!(record["verification"]["pass"], false);
    assert!(record["verified_staging"].is_null());
}

#[test]
fn cli_fragmented_subtitles_match_media_verdict() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("fragmented.mp4");
    let out = execute(
        Command::new("ffmpeg")
            .args(["-nostdin", "-y", "-loglevel", "error", "-i"])
            .arg(fixtures().dir.path().join("f08_mp4_en_fr.mp4"))
            .args([
                "-map",
                "0",
                "-c",
                "copy",
                "-movflags",
                "frag_keyframe+empty_moov+default_base_moof",
            ])
            .arg(&src),
        Duration::from_secs(60),
    );
    assert!(out.success, "{}", out.stderr);
    let meta = temp.path().join("meta.json");
    fs::write(
        &meta,
        br#"{"fixtures":[{"name":"fragmented.mp4","original":"en","anime":false}]}"#,
    )
    .unwrap();
    let staging = temp.path().join("staging");
    let remux_out = command("remux", Some(&meta), Some(&staging), &src);
    assert!(remux_out.success, "{}", remux_out.stderr);
    let p = torcul_policy::plan(
        &torcul_media::probe(&src).unwrap(),
        &torcul_policy::TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    )
    .unwrap();
    let report = torcul_media::verify(&src, &p, &staging.join("fragmented.mp4")).unwrap();
    let verify_out = command("verify", None, Some(&staging), &src);
    assert_eq!(verify_out.success, report.pass);
}

#[test]
fn cli_refuses_hard_linked_source() {
    let (temp, src) = copy_fixture("f07_default_dropped.mkv");
    let link = temp.path().join("hardlink.mkv");
    fs::hard_link(&src, &link).unwrap();
    let staging = temp.path().join("staging");
    let out = command(
        "remux",
        Some(&fixtures().dir.path().join("manifest.json")),
        Some(&staging),
        &src,
    );
    assert!(!out.success);
    assert!(out.stderr.contains("hard links"), "{}", out.stderr);
    assert!(!staging.exists());
}

#[test]
fn cli_plan_json_rejects_colliding_destinations() {
    let (_temp, src) = copy_fixture("f01_movie_en_de_extra.mkv");
    let src_bytes = fs::read(&src).unwrap();
    let work = tempfile::tempdir().unwrap();
    let meta = work.path().join("meta.json");
    fs::copy(fixtures().dir.path().join("manifest.json"), &meta).unwrap();
    let meta_bytes = fs::read(&meta).unwrap();

    let out_input = cli(&[
        OsStr::new("plan"),
        OsStr::new("--meta"),
        meta.as_os_str(),
        OsStr::new("--json"),
        src.as_os_str(),
        src.as_os_str(),
    ]);
    assert!(!out_input.success);
    assert!(
        out_input.stderr.contains(&src.display().to_string()),
        "{}",
        out_input.stderr
    );
    assert_eq!(fs::read(&src).unwrap(), src_bytes);

    let out_meta = cli(&[
        OsStr::new("plan"),
        OsStr::new("--meta"),
        meta.as_os_str(),
        OsStr::new("--json"),
        meta.as_os_str(),
        src.as_os_str(),
    ]);
    assert!(!out_meta.success);
    assert!(
        out_meta.stderr.contains(&meta.display().to_string()),
        "{}",
        out_meta.stderr
    );
    assert_eq!(fs::read(&meta).unwrap(), meta_bytes);
}

#[test]
fn cli_plan_json_missing_parent_fails_cleanly() {
    let work = tempfile::tempdir().unwrap();
    let meta = fixtures().dir.path().join("manifest.json");
    let src = fixtures().dir.path().join("f01_movie_en_de_extra.mkv");
    let missing_parent = work.path().join("no_such_dir").join("plan.json");

    let out = cli(&[
        OsStr::new("plan"),
        OsStr::new("--meta"),
        meta.as_os_str(),
        OsStr::new("--json"),
        missing_parent.as_os_str(),
        src.as_os_str(),
    ]);
    assert!(!out.success);
    assert!(!out.stderr.contains("panicked"), "{}", out.stderr);
    assert!(
        out.stderr.contains(&missing_parent.display().to_string()),
        "{}",
        out.stderr
    );
}

#[cfg(unix)]
#[test]
fn cli_plan_json_rejects_symlink_to_input() {
    let (_temp, src) = copy_fixture("f01_movie_en_de_extra.mkv");
    let src_bytes = fs::read(&src).unwrap();
    let work = tempfile::tempdir().unwrap();
    let link = work.path().join("alias.json");
    std::os::unix::fs::symlink(&src, &link).unwrap();
    let meta = fixtures().dir.path().join("manifest.json");

    let out = cli(&[
        OsStr::new("plan"),
        OsStr::new("--meta"),
        meta.as_os_str(),
        OsStr::new("--json"),
        link.as_os_str(),
        src.as_os_str(),
    ]);
    assert!(!out.success);
    assert!(
        out.stderr.contains(&link.display().to_string()),
        "{}",
        out.stderr
    );
    assert_eq!(fs::read(&src).unwrap(), src_bytes);
}

#[test]
fn cli_plan_failure_does_not_write_json() {
    let work = tempfile::tempdir().unwrap();
    let meta = work.path().join("empty_meta.json");
    fs::write(&meta, br#"{"fixtures":[]}"#).unwrap();
    let json = work.path().join("plan.json");
    let src = fixtures().dir.path().join("f01_movie_en_de_extra.mkv");

    let out = cli(&[
        OsStr::new("plan"),
        OsStr::new("--meta"),
        meta.as_os_str(),
        OsStr::new("--json"),
        json.as_os_str(),
        src.as_os_str(),
    ]);
    assert!(!out.success);
    assert!(out.stdout.contains("no metadata entry"), "{}", out.stdout);
    assert!(!json.exists(), "failed plan must not write JSON");
}

#[cfg(unix)]
#[test]
fn cli_symlinked_staging_dir_remux_and_verify() {
    let (temp, src) = copy_fixture("f07_default_dropped.mkv");
    let real_staging = temp.path().join("real_staging");
    fs::create_dir(&real_staging).unwrap();
    let link_staging = temp.path().join("staging_link");
    std::os::unix::fs::symlink(&real_staging, &link_staging).unwrap();
    let meta = fixtures().dir.path().join("manifest.json");

    let remux_out = command("remux", Some(&meta), Some(&link_staging), &src);
    assert!(remux_out.success, "{}", remux_out.stderr);
    assert!(real_staging.join("f07_default_dropped.mkv").is_file());
    assert!(
        real_staging
            .join("f07_default_dropped.mkv.torcul.json")
            .is_file()
    );

    let verify_out = command("verify", None, Some(&link_staging), &src);
    assert!(verify_out.success, "{}", verify_out.stderr);

    let broken_link = temp.path().join("broken_staging");
    std::os::unix::fs::symlink(temp.path().join("missing_target"), &broken_link).unwrap();
    let broken_out = command("remux", Some(&meta), Some(&broken_link), &src);
    assert!(!broken_out.success);
    assert!(
        !broken_out.stderr.contains("panicked"),
        "{}",
        broken_out.stderr
    );
}

#[cfg(target_os = "linux")]
fn run_with_dev_full_stdout(args: &[&OsStr]) -> (bool, String) {
    let out = execute_to(
        Command::new(env!("CARGO_BIN_EXE_torcul")).args(args),
        Path::new("/dev/full"),
        Duration::from_secs(600),
    );
    (out.success, out.stderr)
}

#[cfg(target_os = "linux")]
#[test]
fn cli_dev_full_stdout_exits_nonzero_without_panic() {
    let meta = fixtures().dir.path().join("manifest.json");
    let sample = fixtures().dir.path().join("f06_already_clean.mkv");

    let (plan_ok, plan_err) = run_with_dev_full_stdout(&[
        OsStr::new("plan"),
        OsStr::new("--meta"),
        meta.as_os_str(),
        sample.as_os_str(),
    ]);
    assert!(!plan_ok);
    assert!(!plan_err.contains("panicked"), "{plan_err}");

    let (temp, src) = copy_fixture("f06_already_clean.mkv");
    let staging = temp.path().join("staging");
    let (remux_ok, remux_err) = run_with_dev_full_stdout(&[
        OsStr::new("remux"),
        OsStr::new("--meta"),
        meta.as_os_str(),
        OsStr::new("--staging"),
        staging.as_os_str(),
        src.as_os_str(),
    ]);
    assert!(!remux_ok);
    assert!(!remux_err.contains("panicked"), "{remux_err}");
}
