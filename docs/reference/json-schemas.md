---
# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
description: JSON Schema Draft 2020-12 contracts for torcul job requests, job records, staging sidecars and plan reports.
---

# JSON Schemas

torcul publishes [JSON Schema Draft 2020-12](https://json-schema.org/draft/2020-12/schema)
contracts for every JSON document it reads from callers or writes to disk. All
four schema files live under `schemas/` and are generated from the Rust types
through `schemars`. [Schema subcommand][schema-cmd] · [Schema tests][schema-test]

## Regenerate and check schemas

From the repository root, run the `schema` subcommand to write all four schema
files atomically into `schemas/`:

```sh
cargo run -p torcul-cli -- schema --out schemas
```

Sample version 1 documents live in
[`schemas/examples/job-request-1.json`][example-request] and
[`schemas/examples/job-1.json`][example-job]. The integration suite checks that
checked-in schemas match generator output byte for byte, conform to the Draft
2020-12 meta-schema, and accept or reject positive, negative and boundary
documents. [Schema tests][schema-test] · [CI drift step][ci]

## Published schema files

| File | Schema `title` | Rust root type | Writer and role |
| :--- | :--- | :--- | :--- |
| [`schemas/job-request-1.schema.json`][job-request-schema] | `torcul/job-request/1` | [`torcul_job::JobRequest`][job-crate] | Written by an external caller (such as SnatchArr) to trigger a `plan`, `stage` or `apply` job ([ADR-0002, decision 9][decision-9]) |
| [`schemas/job-1.schema.json`][job-schema] | `torcul/job/1` | [`torcul_job::Job`][job-crate] | Written by the planned `torcul` job runner at `<staging_root>/<id>/job.json` to record state transitions, fingerprints, plan and verification verdict ([ADR-0002, decision 12][decision-12]) |
| [`schemas/staging-record-1.schema.json`][staging-schema] | `torcul/staging-record/1` | [`Record`][staging-src] | Written by `torcul remux` and updated by `torcul verify` at `<output>.torcul.json` ([Track-policy CLI][cli-guide] · [CLI tests][cli-test]) |
| [`schemas/plan-report.schema.json`][plan-schema] | `torcul/plan-report` | `Vec<`[`FileReport`][report-src]`>` | Written by `torcul plan --json <path>` only when every file plans: each entry holds a plan and `error: null`; any planning error stops the command before the write ([Track-policy CLI][cli-guide] · [CLI tests][cli-test]) |

## Versioning and compatibility rules

- **Unknown versions are refused.** `job-request-1.schema.json` requires
  `schema: "torcul/job-request/1"` and `job-1.schema.json` requires
  `schema: "torcul/job/1"`; `serde` deserialization and JSON Schema validation
  both reject any other identifier. `staging-record-1.schema.json` sets
  `const: 1` on `version`, and `torcul verify` rejects other numbers with
  `"unsupported sidecar version"`. [Job types][job-crate] ·
  [Sidecar loader][staging-src] · [Schema tests][schema-test]
- **Requests reject unknown fields.** `JobRequest`, `Rule`, `TrackRule` and
  `RequestTitle` carry `#[serde(deny_unknown_fields)]` (`additionalProperties: false` in JSON
  Schema), so misspelled or unsupported request keys fail before media files
  are touched. `rollback_days` is optional in the request and defaults to `7`
  (`0..=65535`). [Job types][job-crate] · [ADR-0002, decision 9][decision-9]
- **Version 1 records grow only by optional fields.** `Job` omits
  `deny_unknown_fields` so `torcul/job/1` can gain optional fields without
  breaking readers. Any incompatible change to field types, meanings or
  required keys increments the trailing version number (for example
  `torcul/job/2`). [Job types][job-crate] · [ADR-0002, decision 9][decision-9]
- **Scalar invariants are encoded in the schema.** `Fingerprint.sha256` matches
  `^sha256:[0-9a-f]{64}$`, `Job.id` matches `^[0-9a-f]{32}$`, and every `u32`
  and `u64` field carries its exact `maximum`. `TrackRule.languages` requires
  `minItems: 1`, and `serde` refuses an empty list too. [Media I/O][media-io] ·
  [Job types][job-crate] · [Schema generator][schema-cmd] ·
  [Schema tests][schema-test]
- **Timestamps are checked without format validation.** `Transition.at`
  matches an RFC 3339 UTC pattern (`2026-11-26T17:00:07Z`, optional
  fractional seconds), so any Draft 2020-12 validator refuses a malformed
  shape. The pattern checks the shape only; enable format validation (in
  the `jsonschema` crate, `should_validate_formats(true)`) to also refuse
  impossible dates such as month 13 through `format: "date-time"`.
  [Job types][job-crate] · [Schema tests][schema-test]
- **Some fields depend on the job state.** In `applying` and `rolling_back`,
  `source_fingerprint` and `staged_fingerprint` must be set, because a
  restart reads both inodes from them; `rolling_back` also needs
  `rollback_path`. [Job types][job-crate] ·
  [ADR-0002, decision 15][decision-15]
- **A plan-report entry has exactly one outcome.** Both `plan` and `error`
  are always present; exactly one of them is `null`. [Plan report][report-src]
  · [Schema tests][schema-test]

## Drift checks

Never edit `schemas/*.schema.json` by hand. Update the Rust type or its
`#[schemars(...)]` attribute, then run
`cargo run -p torcul-cli -- schema --out schemas`. Both the local
`generated_schemas_match_checked_in_files` test and the `Check JSON Schema
drift` step in CI fail if any schema file is stale or untracked.
[Schema generator][schema-cmd] · [Schema tests][schema-test] · [CI drift step][ci]

[schema-cmd]: ../../crates/torcul-cli/src/schema.rs
[schema-test]: ../../crates/torcul-cli/tests/schema.rs
[ci]: ../../.github/workflows/ci.yml
[example-request]: ../../schemas/examples/job-request-1.json
[example-job]: ../../schemas/examples/job-1.json
[job-request-schema]: ../../schemas/job-request-1.schema.json
[job-schema]: ../../schemas/job-1.schema.json
[staging-schema]: ../../schemas/staging-record-1.schema.json
[plan-schema]: ../../schemas/plan-report.schema.json
[job-crate]: ../../crates/torcul-job/src/lib.rs
[decision-9]: ../adr/0002-pipeline-architecture.md#decision-9-trigger-and-state-boundary
[decision-12]: ../adr/0002-pipeline-architecture.md#decision-12-staging-output
[decision-15]: ../adr/0002-pipeline-architecture.md#decision-15-restart-and-yield
[staging-src]: ../../crates/torcul-cli/src/staging.rs
[report-src]: ../../crates/torcul-cli/src/report.rs
[media-io]: ../../crates/torcul-media/src/io.rs
[cli-guide]: ../guides/track-policy-cli.md
[cli-test]: ../../crates/torcul-cli/tests/cli.rs
