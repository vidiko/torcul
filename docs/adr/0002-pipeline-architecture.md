# ADR-0002: torcul runs its media pipeline in process from Rust, through libav* and direct SVT-AV1

- **Status**: Accepted; supersedes the "FFmpeg argument builder" part of
  [ADR-0001](0001-vmafx-integration.md) decision 4
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
| Probe 11 files | in process 0.073 s CPU; `ffprobe` per file 0.520 s (7x); `gst-discoverer-1.0` per file 4.073 s (56x) |
| Remux 120 s 1080p (95 MB), keep 9 of 13 streams | in process 0.139 s; `mkvmerge` 0.156 s; `ffmpeg -c copy` 0.187 s; all limited by I/O |
| Remux fidelity | in process and `mkvmerge` keep the cover attachment and the source default flags; `ffmpeg` turns the cover into a video track and adds default flags; the GStreamer pipeline keeps only the tracks it links and flags every track default |
| AV1 encode, 240 frames 1080p, SVT-AV1 preset 8 | CLI 10.75 s CPU, 2.78 s wall; through libavcodec 11.43 s CPU (+6 %), 3.36 s wall (+21 %), same output size; GStreamer 11.00 s CPU |
| HEVC encode, same input, x265 preset fast | CLI 19.09 s CPU, 6.09 s wall; through libavcodec 18.02 s CPU (-6 %), 4.62 s wall; GStreamer 17.83 s CPU |
| AV1 decode, dav1d, 8 threads | CLI 0.90 s; through libavcodec 0.99 s (+10 %) |

The evidence file lists an earlier run of the same cases. Between the two
runs the x265 difference changed sign (+5.0 % through libavcodec there, -6 %
here), so for x265 the wrapper costs nothing this host can measure. The
SVT-AV1 difference kept its sign (+6.6 % there, +6 % here), and so did dav1d
(+29 % there, +10 % here). Starting a process costs about 0.05 s CPU (0.520 s
for 11 `ffprobe` runs), which explains part of the dav1d difference on a
decode under one second but little of the 0.67 s SVT-AV1 difference. For
probe and remux, what differs is the cost of starting processes and whether
the tool keeps the file as it was.

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
4. **Verify before write, every time.** A remux passes when, for every kept
   stream, the packet count, the sum of packet sizes and a SHA-256 over the
   packet payloads equal the source, and kind, codec, language, title,
   dispositions, chapters and attachments match the plan. An encode passes
   when its vmafx full-reference score, with the decoded library original as
   the reference, meets the job's target (ADR-0001: in process through
   `libvmafx`, reference decoded once and reused across trials); otherwise the
   original is kept (issue #3). `apply` refuses unless the verdict passed and
   the source fingerprint is unchanged, then renames atomically on the same
   filesystem.
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
  times the in-process probe, and the tool changes default flags and turns
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
  torcul can take it. Hardware choices stay unmeasured until the cluster
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
