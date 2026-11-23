# ADR-0002: torcul runs its media pipeline in process from Rust, through libav* and direct SVT-AV1

- **Status**: Accepted; supersedes the "FFmpeg argument builder" part of
  [ADR-0001](0001-vmafx-integration.md) decision 4; extended on 2026-11-23 by
  decisions 8 to 17 (execution ownership, apply and rollback)
- **Date**: 2026-11-17
- **Authors**: Lusoris

## Context

torcul (see [ADR-0001](0001-vmafx-integration.md) for the name) changes files
in a video library, often many at once, so every change has to be planned,
checked and reversible until it is proven. The pipeline has these stages:

```text
probe -> plan -> demux -> [decode -> filter -> encode] -> mux -> verify -> apply
                          '---- skipped for a remux ----'
```

- **probe** reads the container, streams, languages, dispositions, chapters
  and attachments of a file.
- **plan** decides what to keep, drop or re-encode. It is pure logic with no
  media library behind it.
- **demux, mux** copy packets between containers. A remux stops here: no
  decoding, no quality loss.
- **decode, filter, encode** apply when a video track is re-encoded (geometry,
  HDR, deinterlace, codec change; issues #2 to #8). An encode job writes a
  video-only file (issue #2), which a mux step joins with the kept tracks.
- **verify** proves the output before anything touches the library: a remux
  must be a lossless copy of the kept streams; an encode is scored
  full-reference with vmafx against the library original.
- **apply** replaces the library file only after a verify pass, atomically.

The first feature is the interim library track policy, ledger id
`praetor-decided-media-track-policy` (operator decision of 2026-11-17,
paraphrased): keep English and German audio, plus the original language when
neither is present (anime: original, English and German); keep English and
German subtitles in all variants; drop the rest only after a verified pilot.
It needs only probe, plan, demux, mux, verify and apply. The operator asked on
2026-11-17 (paraphrased) for this work to start because flows built on the
`ffmpeg` command-line tool cost too much time.

The fleet's own-apps rule asks for low-level parts as Rust crates and Go only
where it serves (ADR-0001, decision 4). Three ways to drive the media stages
were compared:

- **Direct codec libraries**: dav1d, SVT-AV1 and x265 called through their own
  C APIs, plus Intel Quick Sync Video, VA-API or Vulkan Video for the Arc B60
  and B580 cards in the cluster.
- **libav\***: FFmpeg's libraries (libavformat, libavcodec, libavfilter,
  libavutil) called in process, which wrap the same codec libraries and the
  same hardware APIs.
- **GStreamer**: pipelines of elements (`matroskademux`, `svtav1enc`,
  `x265enc` and so on).

The command-line tools (`ffmpeg`, `ffprobe`, `gst-launch-1.0`, `mkvmerge`)
were measured beside them, since they are what the current flows use.

### Measurements

All figures come from [`tools/bench/backends.py`](../../tools/bench/backends.py)
on the workstation, pinned to 8 CPUs, median of 3 to 5 runs. The full table,
host facts, method and the in-process code are in
[`evidence/0002-backends.md`](evidence/0002-backends.md), with raw data in
[`evidence/0002-backends.json`](evidence/0002-backends.json). CPU time is the
figure compared; the host was under load from other builds.

| Question | Result |
| :--- | :--- |
| Probe 11 files | in process 0.073 s CPU; `ffprobe` per file 0.520 s (7x); `gst-discoverer-1.0` per file 4.073 s (56x). Second run with `ffprobe` printing only the four fields the in-process probe prints: 0.470 s against 0.058 s (8x) |
| Remux 120 s 1080p (95 MB), keep 9 of 13 streams | in process 0.139 s; `mkvmerge` 0.156 s; `ffmpeg -c copy` 0.187 s; all limited by I/O |
| Remux fidelity | in process and `mkvmerge` keep the cover attachment and the source default flags; `ffmpeg` turns the cover into a video track and adds default flags; the GStreamer pipeline keeps only the tracks it links and flags every track default |
| AV1 encode, 240 frames 1080p, SVT-AV1 preset 8 | CLI 10.75 s CPU, 2.78 s wall; through libavcodec 11.43 s CPU (+6 %), 3.36 s wall (+21 %), same output size; GStreamer 11.00 s CPU |
| HEVC encode, same input, x265 preset fast | CLI 19.09 s CPU, 6.09 s wall; through libavcodec 18.02 s CPU (-6 %), 4.62 s wall; GStreamer 17.83 s CPU |
| AV1 decode, dav1d, 8 threads | CLI 0.90 s; through libavcodec 0.99 s (+10 %) |

The evidence file lists an earlier run of the same cases. Between the two
runs the x265 difference changed sign (+5.0 % through libavcodec there, -6 %
here), so for x265 the wrapper costs nothing this host can measure. The
SVT-AV1 difference kept its sign (+6.6 % there, +6 % here), and so did dav1d
(+29 % there, +10 % here). Both probes call the same libavformat functions,
and the four-field `ffprobe` row prints what the in-process probe prints, so
the probe gap is the cost of starting a process and loading FFmpeg: about
0.04 s CPU per file ((0.470 s - 0.058 s) / 11). That explains part of the
0.09 s dav1d difference on a decode under one second but little of the
0.67 s SVT-AV1 difference. For probe and remux, what differs is the cost of
starting processes and whether the tool keeps the file as it was.

Not measured: hardware encode and decode on the Arc cards. The workstation
has an NVIDIA card; the Arc cards are only in the cluster. The FFmpeg build
used here has `av1_qsv`, `hevc_qsv`, `av1_vaapi`, `hevc_vaapi`, `av1_vulkan`
and `hevc_vulkan` encoders (`ffmpeg -hide_banner -encoders`); the GStreamer
install has no hardware elements to compare against.

Binding crates on crates.io (`cargo search`, 2026-11-17): `ffmpeg-next`
9.1.0 tracks FFmpeg 9.1 but is in maintenance-only mode (its README: it is
kept compiling against FFmpeg 3.4 and later, and new FFmpeg APIs may need a
pull request from users); `gstreamer` 0.25.4 is the GStreamer project's own
binding. Among the direct libraries only dav1d has a binding past 0.1
(`dav1d` 0.11.1); SVT-AV1 has `svt-av1-enc` 0.1.2 and x265 has `x265-sys`
0.1.0, so torcul would write and maintain most of that FFI itself.

## Decision

1. **libav\* in process from Rust, with SVT-AV1 called directly for AV1
   encode.**
   - torcul calls libavformat, libavcodec, libavfilter and libavutil through
     the `ffmpeg-next` crate, and through `ffmpeg-sys-next` only for fields
     the safe API lacks; each `unsafe` block carries a `// SAFETY:` proof
     (HISS-09). libav\* covers probe, demux, mux, decode and filter, and
     every encoder except software AV1: libx265 for HEVC (the x265 tool
     measured no cheaper), librav1e for issue #4, and the hardware encoders
     (`*_qsv`, `*_vaapi`, `*_vulkan`), so a hardware choice stays a codec
     name, not a second framework.
   - Software AV1 encode calls the SVT-AV1 encoder library (`libSvtAv1Enc`,
     header `EbSvtAv1Enc.h`) directly, in process. Its own tool cost 6 % less
     CPU and 17 % less wall time than the same encode through libsvtav1, and
     the CPU difference kept its sign across both runs. Frames decoded and
     filtered by libav\* go to SVT-AV1 as picture planes; the AV1 packets it
     returns are muxed by libavformat like any other encoder's.
   - The binding is a maintained crate if one tracks the installed SVT-AV1
     release (the candidate is `shiguredo_svt_av1` 2026.2.0, Apache-2.0,
     released 2026-08-12), otherwise `bindgen` bindings against
     `EbSvtAv1Enc.h` in a torcul `-sys` crate. The encode slice records the
     choice and measures the in-process path against libsvtav1, since the
     bench measured the SVT-AV1 tool, not torcul's own calls.
   - **One encode trait.** Both backends implement one `Encoder` trait:
     decoded frames in, packets and stream parameters out, settings from the
     encoder settings crate. One shared test suite runs its positive,
     negative and boundary cases against every backend, and its AV1 cases
     run the direct SVT-AV1 path.
   - The "FFmpeg argument builder" crate of ADR-0001 decision 4 becomes this
     encoder settings crate. It produces settings per backend (a libavcodec
     options dictionary, or SVT-AV1 configuration fields), not command lines.
2. **No media command-line tool at run time.** torcul never starts `ffmpeg`,
   `ffprobe`, `gst-launch-1.0` or `mkvmerge`. Two exceptions: tests may use
   the `ffmpeg` tool to generate synthetic fixtures
   ([`tools/fixtures/track_fixtures.py`](../../tools/fixtures/track_fixtures.py)),
   and scoring may call the `vmaf` binary until `libvmafx` ships (the phase 0
   seam of ADR-0001 decision 5).
3. **Remux keeps the file as it was, except for the planned change.** Streams
   are copied packet for packet, with metadata, dispositions, chapters,
   global metadata and attachments. A Matroska cover image, which libavformat
   exposes as an `attached_pic` video stream, is written back as an
   attachment (the conversion is shown in the evidence file). The output
   container is the input container: MP4 stays MP4.
4. **Verify before write, every time.**
   - A remux passes when every kept stream equals its source stream in:
     - the packet count, and per packet the payload size, the key flag, and
       pts, dts and duration rescaled to a common time base (equal within one
       tick of the coarser base);
     - a SHA-256 over the packet payloads;
     - the codec parameters (codec id, profile, level, picture geometry or
       sample format, frame or sample rate, channel layout) and extradata;
     - the stream and packet side data that change decoding or display
       (display matrix, mastering display, content light level, Dolby Vision
       configuration).

     Kind, language, title, dispositions, chapters and attachments must match
     the plan.
   - An encode passes when its vmafx full-reference score, with the decoded
     library original as the reference, meets the job's target (ADR-0001: in
     process through `libvmafx`, reference decoded once and reused across
     trials); otherwise the original is kept (issue #3).
   - `apply` holds an exclusive advisory lock (`flock`) on a lock file beside
     the source from its first check to its last step, and every torcul job
     that writes a library file takes the same lock. Under the lock it:
     1. refuses unless the verdict passed;
     2. gives the staged output the source's owner, group, mode, ACLs and
        extended attributes, reads them back, and refuses if any differ;
     3. refuses unless the source fingerprint taken at plan time (device,
        inode, size, modification time, content hash) is unchanged, and
        refuses a source with more than one hard link, whose other links
        would keep the old file;
     4. swaps the staged output and the source in one step with
        `renameat2(RENAME_EXCHANGE)` on the same filesystem, so no reader
        sees a missing or partial file;
     5. checks the swapped-out original against the fingerprint again and
        swaps back if it changed: a writer outside torcul ignores the
        advisory lock, and this catches a change it made before the swap;
     6. keeps the original at the staging path as the rollback copy until
        the report is accepted.

     A filesystem that rejects `RENAME_EXCHANGE` (`EINVAL`, man 2 rename)
     fails the apply; there is no plain-rename fallback.
5. **Crates.** `torcul-policy` (pure planning, no FFmpeg), `torcul-media`
   (probe, remux, verify over libav\*) and `torcul-cli` (the `torcul`
   binary: `plan`, `remux`, `verify`, `apply`) come first. An encode crate
   with the `Encoder` trait, its two backends and their shared tests comes
   next; hardware probe and scoring crates follow with issues #3 to #9. The
   Go service layer
   of ADR-0001 waits for its own cgo measurement; nothing in this record needs
   Go.
6. **Cluster job shape.** Private media never leaves the cluster.
   - First phase: a Kubernetes Job runs `torcul` over a library subtree, next
     to the library storage. `plan` writes the dry-run report; `remux` writes
     to a staging directory on the same filesystem as the library; `verify`
     checks it; `apply` runs only after the report and a pilot set were
     reviewed.
   - Later phase: the service runs the bounded pools of issue #6: `remux_io`
     (I/O bound, never starved by encodes), `encode_cpu` and `encode_gpu`.
   - A GPU job asks for the Intel device plugin resource and uses the one
     render node the plugin gives it. torcul reads that node's PCI address
     from sysfs, opens the libav\* hardware device on that node and the vmafx
     device with the same PCI address (ADR-0001, decision 2). Jobs name no
     node, no card and no device index; the cluster's arbiter places them on
     measured free memory. Library jobs are batch work and yield to live
     transcodes.
7. **Licensing.** torcul's source stays EUPL-1.2. FFmpeg built with libx265
   is GPL: the workstation build has `--enable-gpl --enable-version3`
   (`ffmpeg -buildconf`) and states GPL version 3 or later (`ffmpeg -L`). A
   torcul binary linked against such a build is distributed under GPL-3.0,
   using the compatibility clause of EUPL-1.2 ([`LICENSE`](../../LICENSE),
   Article 5, and the Appendix that lists GPL v2 and v3). Images ship the
   licence notices of FFmpeg and each codec: `ffmpeg-next` WTFPL (its
   `Cargo.toml`), dav1d BSD-2-Clause, SVT-AV1 (linked directly and through
   FFmpeg) BSD-3-Clause-Clear with the Alliance for Open Media Patent
   License 1.0, x265 GPL-2.0-or-later
   (workstation package metadata and `/usr/share/licenses/svt-av1/`).

Rejected:

- **Direct codec libraries for every stage.** Only SVT-AV1 encode showed a
  gain that is not noise. x265 measured no cheaper than through libavcodec.
  dav1d's tool was cheaper in both runs, but on decodes under one second,
  where process start is a large share. libavcodec also picks the decoder
  for every input codec, which dav1d alone cannot (it decodes AV1 only), and
  its frames feed libavfilter directly. A direct path per codec would be FFI
  torcul writes and maintains for each library. Options libavcodec does not
  name still reach libx265 through `x265-params`.
- **GStreamer.** Probing cost 56 times the in-process figure; the remux
  pipeline needs a parser element per codec (`h264parse`, `aacparse` in the
  bench), lost the cover and flagged every track default; its encoders
  measured the same as the others, so it adds a framework without a gain.
- **The `ffmpeg` and `ffprobe` tools.** Spawning a process per file costs 7
  to 8 times the in-process probe, also when `ffprobe` prints no more than
  the in-process probe does, and the tool changes default flags and turns
  cover images into video tracks on a plain `-c copy`.
- **mkvmerge.** It remuxes faithfully and was the fidelity bar here, but it
  writes Matroska only (MP4 files would change container) and runs as a
  subprocess.

## Consequences

- **Positive**: one library for every stage and every codec except software
  AV1 encode, which skips the libsvtav1 overhead measured here; one encode
  trait keeps the two encode backends interchangeable and tested alike; no
  process per file; remux output differs from the source only
  where the plan says so, and the verify stage proves it per packet; the
  crates fit the own-apps rule and can be reused by Aegis-OS (ADR-0001).
- **Negative**: torcul links FFmpeg, so a GPL FFmpeg build makes torcul
  binaries GPL-3.0 when distributed, and the system FFmpeg version becomes a
  build input (CI installs the FFmpeg development packages). `ffmpeg-next`
  is maintenance-only, so new FFmpeg APIs may reach torcul through
  `ffmpeg-sys-next` and `unsafe` first, or need a pull request upstream. Some
  libav\* behaviour has to be corrected in torcul, such as the cover image
  conversion in decision 3. AV1 encode has a second binding to keep current:
  an SVT-AV1 release that changes its API needs a binding update before
  torcul can take it. `apply` needs a filesystem that supports
  `RENAME_EXCHANGE`; the cluster library storage has to be checked for it
  before the first apply. Hardware choices stay unmeasured until the cluster
  measurement below lands.
- **Follow-up** (issue #11): measure hardware encode and decode on the
  cluster's Arc B60 and B580 (Quick Sync Video against VA-API against Vulkan Video, AV1 and
  HEVC, speed and vmafx score at equal size, and a decode, scale and encode
  chain that stays on the card); the result feeds codec tiering (issue #4)
  and the hardware probe (issue #5).
- **Revisit when**:
  - torcul's in-process SVT-AV1 path measures no cheaper than libsvtav1 on
    cluster workloads (AV1 encode then moves back into libavcodec);
  - the SVT-AV1 binding cannot follow an SVT-AV1 release that torcul needs;
  - another direct library (x265, dav1d) measures more than 10 % cheaper than
    its libavcodec wrapper on library-length cluster workloads;
  - an encoder control is needed that neither libavcodec nor the SVT-AV1 API
    can pass through;
  - `ffmpeg-next` stops compiling against a current FFmpeg release;
  - live streaming becomes a goal, which is GStreamer's strength.

## References

- Order `torcul-kickoff-20261117`, operator direction of 2026-11-17
  (paraphrased): start the encode and decode project now; `ffmpeg`
  tool-based flows cost too much time. Tracking issue #10.
- Ledger id `torcul-backend-libav-inprocess`, operator answer of 2026-11-17
  on the media backend (paraphrased): libav\* in process for probe, remux and
  decode; SVT-AV1 called directly for AV1 encode; HEVC on libx265 through
  libav\*; the encode path behind one trait so both backends share tests.
- Ledger id `praetor-decided-media-track-policy`: interim library track
  policy (first feature); missing subtitles are fetched or generated in the
  cluster later. Order `twenty-watts-media-track-checks-20261117`: research
  into cheap language and subtitle checks for the check stage that takes over
  the tracks the plan flags.
- [ADR-0001](0001-vmafx-integration.md): vmafx in process through
  `libvmafx`, GPU lease matched by PCI address.
- Issues #2 to #9: encode job slices (platform API, auto mode, codec
  tiering, hardware probe, pools, geometry, HDR, scoring surface).
- FFmpeg n9.0.2, SVT-AV1 v4.2.0, x265 4.3, dav1d 1.5.4, GStreamer 1.28.9,
  MKVToolNix v103.0, `ffmpeg-next` 9.1.0: the versions measured.

## Extension of 2026-11-23: torcul owns media execution, apply and rollback

This section adds decisions 8 to 17. Decisions 1 to 7 above stand as written;
where a decision below makes one of them more precise, it names it. The
interfaces this section relies on are listed, with the commit each was read
at, in [`evidence/0002-contracts.md`](evidence/0002-contracts.md).

### Context of the extension

Two projects had planned to run the same media changes:

- **SnatchArr** (said SNATCH-ar, /ˈsnætʃ.ɑːr/: English *snatch* plus the
  `-arr` suffix of the Sonarr and Radarr family of library managers, the
  "\*arr" tools) searches for missing items and quality upgrades across the
  \*arr tools at a capped rate. Its ADR-0041 (lusoris/SnatchArr,
  `docs/adr/0041-rule-driven-library-processing.md` at `1ae38363f955`,
  accepted 2026-10-26) adds rule-driven library processing in which a
  SnatchArr worker runs `mkvpropedit`, `mkvmerge` and `ffmpeg`, stages to
  `<name>.snatcharr-tmp.<ext>` and moves originals to a `.snatcharr-trash/`
  folder for 7 days.
- torcul's decisions 1 to 7 cover the same stages in process, with no media
  tool at run time (decision 2).

The operator settled this on 2026-11-23 (ledger id
`media-execution-ownership-20261123`, answer c, paraphrased): torcul owns all
media execution, apply and rollback included; SnatchArr keeps the rules, the
\*arr integrations, the queue and the user interface, starts torcul jobs and
shows their state; a SnatchArr record supersedes the execution parts of its
ADR-0041; VMAFx scores; Pelorus keeps processing and telemetry.

**Pelorus** (said pi-LOR-us, /pɪˈlɔːɹəs/) is the GPU sibling of VMAFx: the
Vulkan home for pre-encode filters, frame analysis and encoder telemetry
(VMAFx/pelorus `README.md`). A pelorus is a navigator's sighting instrument,
a compass card without a magnetic needle that takes bearings relative to the
ship's heading, named after Pelorus, said to have been Hannibal's pilot.

### Decision 8: ownership

| Owner | Owns |
| :--- | :--- |
| torcul | probe; plan from the effective rule; demux, decode, filter, encode, mux; staging; verify; apply, rollback and commit; restart; the job record; every lock on a library file |
| SnatchArr | rule hierarchy, resolution, schema and dry-run preview; the module switch; queue and schedule windows; the user interface; the calls to \*arr tools and media servers after a change; the planner protection; the rollback window the user sets |
| VMAFx | full-reference scores and their provenance (ADR-0001) |
| Pelorus | GPU pre-encode filters reached through libavfilter; frame analysis; the encoder telemetry and quality window formats; the encode record format |

- SnatchArr starts no media process and writes no library file. It writes job
  requests, reads torcul's job records and calls torcul's commands.
- There is no in-place header edit (ADR-0041 tier 0). A change to a language,
  a title or a default flag is a remux like any other: staged, verified and
  applied by exchange. It costs a full copy of the file. An in-place edit
  cannot be verified before the library sees it, and a crash during the edit
  leaves a damaged file with no rollback copy.

### Decision 9: trigger and state boundary

In the first phase (decision 6), SnatchArr writes a job request and starts a
Kubernetes Job that runs `torcul` on it. The request has the schema
`torcul/job-request/1`, which torcul owns and versions:

| Field | Meaning |
| :--- | :--- |
| `source` | the library file |
| `mode` | `plan`; `stage` (plan, then remux or encode, then verify); or `apply` (stage, then apply) |
| `rule` | the effective rule SnatchArr resolved: for the track policy the languages to keep, later the encode target |
| `title` | the title facts the plan needs: original language and anime flag (`TitleMeta` in `torcul-policy`) |
| `staging_root` | a directory on the same filesystem as `source` |
| `rollback_days` | how long the rollback copy is kept; default 7, the ADR-0041 retention |

torcul answers with the job record `job.json`, schema `torcul/job/1`, in the
job's directory (decision 12). torcul writes it and SnatchArr only reads it.
It holds the request, the source fingerprint, the plan, the verdict, the
state, and the time and torcul version of every step. The states:

| State | Meaning |
| :--- | :--- |
| `planned` | plan written, nothing staged |
| `unchanged` | the plan changes nothing; final |
| `staged` | output complete in staging |
| `verified` | verdict passed; ready to apply |
| `rejected` | verdict failed; final, library untouched |
| `applying` | exchange in progress |
| `applied` | the library holds the output; the original is the rollback copy |
| `rolling_back` | reverse exchange in progress |
| `rolled_back` | the library holds the original again; final |
| `committed` | rollback copy deleted; final |
| `failed` | a step failed; the record names the step and the error; final |

- `torcul commit <job>` deletes the rollback copy; after it, the job cannot
  be rolled back. A scheduled `torcul commit --expired` run in the cluster
  commits every job whose `rollback_days` have passed since `applied`.
- `torcul rollback <job> <path>` takes the path the applied file has now,
  since an \*arr tool may rename a file after the apply. It refuses unless the
  file at that path matches the applied output's fingerprint (decision 10).
  If an \*arr tool replaced the file with a new download, the rollback copy is
  stale and the job can only be committed.
- Telling the \*arr tools and media servers about an apply or a rollback
  stays with SnatchArr.
- The later service phase (decision 6) replaces the request file with an API.
  That API gets its own record; the states above stay its vocabulary.

### Decision 10: source fingerprint

A fingerprint is the device, the inode, the size, the modification time in
nanoseconds, and `sha256:` followed by 64 hex digits over the whole file. It
is valid only when a `stat` taken before the hash read and one taken after it
agree. The same `sha256:` value is the `input.digest` of a Pelorus encode
record (evidence file).

- torcul refuses a source with more than one hard link (decision 4) and a
  `staging_root` on a different device from the source.
- The fingerprint is taken at plan time and compared after the remux or
  encode, at verify, under the apply lock before the exchange (`stat` first,
  then the full hash), and on the swapped-out original after the exchange
  (decision 4, steps 3 and 5).
- The applied output gets its own fingerprint after the exchange; rollback
  compares against it.

### Decision 11: stream plan

The plan is a pure function of the probe facts, the effective rule and the
title facts (`torcul-policy`, decision 5), and the record stores its version.
It lists the output streams in order. Each names its source stream index and
one action:

- `copy`: packets copied unchanged;
- `encode`: with the encoder settings of decision 1;
- `drop`: with a reason. A track the track policy drops carries
  `"lid": "pending"` until the language-check stage exists (ledger id
  `torcul-decided-mvp-no-apply`).

The plan also fixes each output stream's language, title and dispositions,
the chapters, the attachments (a Matroska cover is written as an attachment,
decision 3) and the global metadata. Every later step derives the plan again
from the stored facts and refuses if the result differs from the stored plan.
A plan that changes nothing ends the job as `unchanged`.

### Decision 12: staging output

Each job has a directory `<staging_root>/<job id>/`. The job id is the first
32 hex digits of the SHA-256 of the source's canonical path. Keying by path
keeps one job per library file: a repeated request finds the running job
instead of starting a second, and two sources with the same file name no
longer meet in one staging directory. The directory holds:

- `job.json`, written to a temporary name, synced with `fsync`, renamed over
  the previous record, followed by an `fsync` of the directory;
- `lock`, the job lock of decision 15;
- `output.<ext>.partial` while the mux runs, renamed to `output.<ext>` after
  an `fsync` once complete; `<ext>` is the source container's extension
  (decision 3).

After the exchange, `output.<ext>` holds the original: it is the rollback copy
until commit, which deletes it and keeps `job.json`. A new request for a file
whose job is `applied` is refused until that job is committed or rolled back.
A new request for a job in a final state replaces it.

### Decision 13: verification

Decision 4's remux checks stand. The verdict also compares the output's
stream, chapter and attachment counts with the plan, and names the SHA-256 of
the staged output and the torcul version that produced it; apply refuses a
staged file whose hash differs from the verdict's. An encode passes when:

- the encoded video has the same frame count and presentation timestamps as
  the source;
- every output frame decodes;
- the VMAFx score meets the job's target (decision 4);
- the output is below the size ceiling the rule sets.

These replace the ADR-0041 checks of duration within 1.0 s, a decode probe and
a perceptual-hash distance. Equal frame counts and timestamps are exact where
a duration tolerance is not, and a cut or a different picture lowers a
full-reference score computed frame by frame against the original.

### Decision 14: apply and rollback

- **Apply** is decision 4 inside the job record. Under the job lock and the
  source lock, torcul records `applying`, runs decision 4 steps 1 to 5, stores
  the applied output's fingerprint and records `applied`.
- **Rollback** runs under the same locks. torcul records `rolling_back`,
  checks the applied file at the given path against the applied fingerprint
  and the rollback copy against the source fingerprint, and exchanges the two
  with `renameat2(RENAME_EXCHANGE)`. The exchange moves names only, so the
  original keeps its inode, owner, mode, ACLs and extended attributes. torcul
  then records `rolled_back` and deletes its own output, which is now at the
  staging path. The original takes the path the applied file had; SnatchArr
  tells the \*arr tool.
- The source lock is decision 4's lock file beside the file being replaced:
  beside the source for an apply, beside the given path for a rollback.

### Decision 15: restart and yield

- Every torcul command on a job first takes `<job dir>/lock` with a
  non-blocking `flock`. If another process holds it, the command exits with
  "busy" and changes nothing. Apply, rollback and commit then take the source
  lock, always after the job lock, so two commands never wait on each other.
- A restarted job continues from its recorded state:
  - a `.partial` output is deleted and the step that wrote it runs again;
  - a source whose fingerprint changed before apply discards the staged
    output, and the job starts again from plan;
  - in `applying` or `rolling_back`, the inode at the library path tells
    which file is where: the output's inode means the exchange happened, the
    original's inode means it did not, and torcul finishes or repeats the
    step to match. Any other inode fails the job and leaves both files where
    they are.
- **Yield.** ADR-0041 paused workers with `SIGSTOP` and `SIGCONT`. A stopped
  process keeps its job lock, its source lock and its GPU lease (decision 6)
  while doing nothing. A torcul job yields by ending instead: SnatchArr stops
  the Job, and the restart rules above resume it from the last completed
  step. An encode slice may add checkpoints inside its step.

### Decision 16: adapters to VMAFx and Pelorus

No adapter is written in this extension; the evidence file is the baseline
they are written against.

- Scoring (issue #9) uses the `vmafx` crate: `Context`, `Model`, `Picture`,
  `Context::read_pictures` and `Context::score_pooled`. Until the safe API
  imports frames, decoded planes are copied into `Picture`. The job record
  stores the score with its provenance where the safe API exposes it
  (`Provenance` is in the crate's generated `api_gen.rs`).
- Each encode builds a `pelorus/encode-record/1` record with `input.digest`
  set to the source fingerprint's hash, through libpelorus's C API
  (`pel_encode_record_build`, `pel_encode_record_hash`) until Pelorus has a
  Rust binding (Pelorus issue 115), and passes the record's digest to VMAFx
  (`vmafx_context_set_encode_record`).
- Encoder telemetry (`PEL_SEC_ENC_TELEMETRY`) and quality windows
  (`PEL_SEC_QUALITY`) are written by the encode slices that need them
  (issues #2 to #9).

### Decision 17: first slice

Issue #10 ships plan, stage and verify for the track policy, and no apply:
ledger id `torcul-decided-mvp-no-apply` keeps every library file unchanged
until the language-check stage exists. Its fixture is one lossless remux of a
Matroska file with several audio and subtitle streams, chapters and
attachments, generated by
[`tools/fixtures/track_fixtures.py`](../../tools/fixtures/track_fixtures.py),
which gains chapters for it. The test asserts the output's stream, chapter
and attachment counts against the plan and runs the decision 4 verify on it.

Apply, rollback and commit (decisions 9 and 14) follow in their own slice,
which waits for:

- the language-check stage running in the cluster;
- a `RENAME_EXCHANGE` check on the cluster library storage (Consequences
  above).

### Consequences of the extension

- **Positive**: one implementation of each media step (HISS-19) instead of a
  torcul one and a SnatchArr worker; every library change, header changes
  included, is staged, verified and reversible; a crash at any step leaves a
  state that restart resolves from the record and the inodes; SnatchArr's
  image needs no media tools.
- **Negative**: a header-only change costs a full copy and a full-file hash
  where an in-place edit rewrites a few bytes; rollback copies hold the size
  of every applied original on the library filesystem for `rollback_days`;
  SnatchArr's library processing waits for torcul's apply slice.
- **Revisit when**:
  - header-only jobs are most of the queue and their copy time measures as
    the limit on throughput;
  - SnatchArr needs a step or a state that the commands and record above do
    not expose;
  - the library filesystem lacks `RENAME_EXCHANGE`;
  - the service API lands (its record takes over decision 9's request file).

### References of the extension

- Order `torcul-media-execution-owner-20261123`; ledger id
  `media-execution-ownership-20261123`, the operator answer of 2026-11-23
  paraphrased in the context above. The SnatchArr side is order
  `snatcharr-media-execution-rescope-20261123`.
- Ledger id `torcul-decided-mvp-no-apply` (2026-11-17): plan, stage and verify
  only until the language-check stage exists.
- [`evidence/0002-contracts.md`](evidence/0002-contracts.md): the `vmafx`
  crate at VMAFx/vmafx `6da90915748d`, Pelorus at VMAFx/pelorus
  `46fb087733e3`, SnatchArr ADR-0041 at lusoris/SnatchArr `1ae38363f955`.
- `man 2 rename` (`RENAME_EXCHANGE`), `man 2 flock`, `man 2 fsync`.
- Wikipedia, "Pelorus (instrument)"; Wiktionary, "pelorus" (pronunciation).
