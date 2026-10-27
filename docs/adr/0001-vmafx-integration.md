# ADR-0001: torcul uses vmafx in process through the libvmafx C ABI

- **Status**: Accepted
- **Date**: 2026-10-27
- **Authors**: Lusoris

## Context

torcul is the video encode service of the Vidiko organisation. The name comes
from Latin *torcular*, a press (wine or oil): input goes in, a pressed product
comes out. It is said TOR-kul.

The encode service was first specified inside VMAFx/vmafx (its ADR-3378, the
encode job API: `auto` rate control only, tiered codec selection with hardware
encoder probing, bounded worker pools with one GPU lease arbiter, source
geometry normalisation, HDR and dynamic HDR preservation). On 2026-10-27 the
operator decided that the service leaves vmafx and lives in its own repository;
vmafx stays the scoring library and tools. torcul needs vmafx for quality:
scoring each encode trial, the `vmaf-tune auto` decision path, and codec
comparison.

The fleet's own-apps rule asks for standalone modules: low-level parts as Rust
crates (reusable later by Aegis-OS, the fleet's Rust appliance platform; the
name is the Greek *aigis*, a shield), the service layer in Go on golusoris
(the fleet's shared Go service toolkit; *Go* plus Latin *lusoris*, "of the
player"), unless a measurement says otherwise.

Three ways for torcul to use vmafx were compared on vmafx commit `3df45a93`:

- **A**: a Rust crate (`vmafx-sys` and `vmafx`, in vmafx under `bindings/rust/`)
  over the `libvmafx.so.1` C ABI (`core/api/vmafx.toml`, `core/include/vmafx/`).
- **B**: the `vmafx-node` worker API over gRPC (`api/vmafx-platform.toml`,
  `cmd/vmafx-node/`).
- **C**: a subprocess of the `vmaf` CLI, or of the Python `vmaf-tune` CLI.

Measured on the workstation (576x324 test pair from vmafx `testdata/`, median
of repeated runs):

| Path | Cost |
| :--- | :--- |
| In process, model load (`vmaf_v0.6.1`), once per worker | 0.199 ms |
| In process, context init + feature registration + teardown per trial | 0.053 ms |
| In process, score of one frame with a warm model | 2.68 ms |
| `vmaf` CLI, `--version` | 0.74 to 0.90 ms |
| `vmaf` CLI, score of one frame | 4.19 ms (built-in model), 5.05 ms (model file) |
| `vmaf` CLI, 48 frames | 22.44 ms (about 3.8 to 4.7 ms fixed cost per call) |
| Python `vmaf-tune --help` | 225 ms |

The in-process figures were taken against `libvmaf.so.3`; `libvmafx.so.1` wraps
the same engine and was not measured separately. Option B was not measured
live; reading its code shows that the unary `Score` call hashes the model file,
writes a temporary file and starts the `vmaf` CLI on every call
(`pkg/libvmaf/run.go` in vmafx), that controller jobs add a 2 s poll interval,
and that `ScoreStream` runs on the CPU only and ships raw planes over gRPC
(about 2.85 GiB per 240-frame 1080p 10-bit sample, 11.4 GiB at 4K).

## Decision

torcul uses vmafx **in process through the `libvmafx.so.1` C ABI, from Rust**
(option A).

1. torcul depends on the `vmafx` Rust crate. vmafx moves `vmafx-sys` from
   `libvmaf.so.3` to `libvmafx.so.1` and generates the bindings from
   `core/api/vmafx.toml` (vmafx issue #2320); the crate exposes devices,
   device info (PCI address), frame import, model loading and the provenance
   record.
2. torcul owns the GPU lease on each host. One lease covers both the FFmpeg
   hardware encode of a job and the `VmafxDevice` that scores its trials;
   devices are matched by PCI address (`VmafxDeviceInfo.pci`) to the DRM
   render nodes FFmpeg uses.
3. The reference sample is decoded once per `auto` search and its frames are
   reused across all bisection trials; the model is loaded once per worker.
4. Low-level parts are standalone Rust crates in this repository (hardware
   probe, FFmpeg argument builder, geometry, HDR, tuning). The service layer
   (job API, queues, worker pools, callers' integration) is Go on golusoris
   unless the cgo boundary costs more than it saves; that is measured when the
   service layer is written.
5. Until vmafx ships `libvmafx.so.1` with the Rust crate, a scoring function
   may call the `vmaf` C binary as a subprocess behind the same interface
   (phase 0). The Python `vmaf-tune` CLI is never called at run time.

Option B is rejected: it is slower than C for single calls, CPU-only and
bandwidth-heavy for streaming, has no device lease, and would split GPU
ownership between two daemons. Option C stays only as the phase 0 seam.

## Consequences

- **Positive**: per-trial overhead drops from about 4 ms to about 0.05 ms plus
  scoring; one arbiter owns each GPU; frames can be shared with the encoder
  without a host copy; provenance comes from the library; the crates are
  reusable by Aegis-OS.
- **Negative**: torcul waits on vmafx for `libvmafx.so.1` packaging and the
  regenerated Rust crate (#2320). While torcul ports encode code from vmafx's
  Go packages (`pkg/hwdevice`, `pkg/ffencode`, `pkg/codecadapter`, `pkg/hdr`,
  `pkg/tune/auto`) there are two implementations; once torcul's crates reach
  parity, vmafx either uses them or retires its copies, so one implementation
  remains.

## References

- Operator decision 2026-10-27 (paraphrased): the encode service becomes its
  own repository in Vidiko and connects to vmafx for quality in whichever way
  the design shows best; low-level parts as Rust crates. Ledger ids
  `vidiko-encode-service-placement-20261027` and
  `vmafx-encode-service-name-r2-20261027-answer` (name torcul, public).
- VMAFx/vmafx ADR-3378 (encode job API), superseded there by the record that
  points here.
- Encode work items moved from VMAFx/vmafx: issues formerly #2986 to #2993.
