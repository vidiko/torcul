# ADR-0002 evidence: interfaces torcul will call, and the execution steps it takes over

Inventory for the extension of 2026-11-23 in
[ADR-0002](../0002-pipeline-architecture.md) (decisions 8 to 17). Taken on
2026-11-23 from the commits named in each heading; every row names the file it
was read from. No adapter exists yet; this file is the baseline the adapters
are written against.

## VMAFx Rust crate (VMAFx/vmafx at `6da90915748d`)

Crate `vmafx` 0.1.0, EUPL-1.2, in `bindings/rust/vmafx/` (last change to the
directory: `7136738155ee`, 2026-11-21). It wraps the raw FFI crate
`vmafx-sys` 0.1.0 (`bindings/rust/vmafx/Cargo.toml`). Its own header calls it
Phase 1 of VMAFx ADR-0929 (`src/lib.rs`, lines 1 to 30).

| Surface | What it offers | Source |
| :--- | :--- | :--- |
| `Context`, `ContextBuilder` | log level, worker threads, frame subsampling, CPU mask; `Context` is not `Send` | `src/context.rs` |
| `Model::from_path` | loads a model `.json` | `src/model.rs` |
| `Picture::new(format, bpc, w, h)` | YUV 4:0:0, 4:2:0, 4:2:2, 4:4:4; 8 to 16 bits per component | `src/picture.rs` |
| `Context::read_pictures(ref, dist, index)`, `flush` | feeds one reference and one distorted frame | `src/context.rs` |
| `Context::score_pooled(model, method, lo, hi)` | pooled score over a frame range; methods mean, min, max, harmonic mean, median, 5th, 10th and 20th percentile | `src/context.rs`, `src/score.rs` |
| `api_gen.rs` (generated from `core/api/vmafx.toml`, C ABI 0.1.14) | backend constants (CPU, CUDA, SYCL, Metal, HIP), `ContextConfig` with a GPU mask, device PCI address, `Provenance` with `encode_record`, `scores_digest` and a canonical `digest` | `src/api_gen.rs` |

Deferred to the raw FFI in `vmafx-sys`: model collections, per-feature scores,
output writers and dmabuf frame import (`README.md`, "API surface (Phase 1)").

## Pelorus (VMAFx/pelorus at `46fb087733e3`, interop ABI 1.6)

C only: the tree holds no `.rs` file at this commit. The interop ABI is
`PELORUS_ABI_MAJOR` 1, `PELORUS_ABI_MINOR` 6
(`libpelorus/include/pelorus/interop.h`).

| Contract | Content | Source |
| :--- | :--- | :--- |
| Encode record `pelorus/encode-record/1` | JSON with encoder name, version, codec and parameters, input (`input.digest` as `sha256:` and 64 hex digits, or `input.id`), filter chain, hardware path; one canonical byte form (an RFC 8785 subset, every scalar a string, reals as `f64:` and 16 hex digits); at most 65536 bytes; `pel_encode_record_build`, `_canonicalize`, `_hash`, `_verify` | `docs/api/encode-record.md`, `libpelorus/include/pelorus/encode_record.h` |
| Encoder telemetry, `PEL_SEC_ENC_TELEMETRY` (ABI 1.4) | one 104-byte record per coded frame: sizes, QP in native and H.264-equivalent scale, PSNR and SSIM, optional block maps, a presence mask | `docs/api/encoder-telemetry.md` |
| Quality window, `PEL_SEC_QUALITY` (ABI 1.6) | one 96-byte window score with half-open PTS range, time base, model hash and metric id; `pel_quality_align` matches windows to telemetry records by exact rational PTS | `docs/api/quality-window.md`, `libpelorus/include/pelorus/quality.h` |

VMAFx stores the encode record's digest in a score's provenance
(`Provenance.encode_record` in the VMAFx `api_gen.rs` above; Pelorus
`docs/api/encode-record.md`, "In one paragraph").

## SnatchArr ADR-0041 (lusoris/SnatchArr at `1ae38363f955`)

`docs/adr/0041-rule-driven-library-processing.md`, Accepted 2026-10-26. No
later SnatchArr record changes it at this commit. The table maps its execution
parts to the torcul decision that now covers them.

| ADR-0041 part | Content there | Now |
| :--- | :--- | :--- |
| Decision 1 | the SnatchArr worker runs `mkvpropedit`, `mkvmerge` and `ffmpeg` | torcul, in process (decisions 1, 2, 8) |
| Decision 3, tiers 0 and 1 | header edit in place, lossless remux | torcul remux, staged and verified (decisions 3, 4, 8) |
| Decision 7, audio | audio transcode and loudness | torcul encode stages (decision 8) |
| Decision 8, OQ-0041-14 | staging `<name>.snatcharr-tmp.<ext>`, pre-replace checks, swap and quarantine with a 7-day retention, rollback | torcul (decisions 4, 12 to 15); the 7-day default is kept as `rollback_days` |
| Decision 9, yield | `SIGSTOP` and `SIGCONT` on worker jobs | stop and restart (decision 15) |

SnatchArr keeps: rule resolution and its schema (decisions 2 and 10 there),
the module switch (decision 1 there), the \*arr and media-server calls
(OQ-0041-15), the queue and schedule windows (decision 9 there), the user
interface (decision 11 there) and the planner protection (decision 12 there).
