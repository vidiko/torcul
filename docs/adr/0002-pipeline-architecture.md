# ADR-0002: torcul runs its media pipeline in process through libav* from Rust

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
| AV1 encode, 240 frames 1080p, SVT-AV1 preset 8 | CLI 10.75 s CPU; through libavcodec 11.43 s (+6 %), same output bytes; GStreamer 11.00 s |
| HEVC encode, same input, x265 preset fast | CLI 19.09 s; through libavcodec 18.02 s (-6 %); GStreamer 17.83 s |
| AV1 decode, dav1d, 8 threads | CLI 0.90 s; through libavcodec 0.99 s (+10 %) |

The encode differences change sign between runs (the evidence file lists an
earlier run), so the wrapper costs nothing measurable next to the encoder
itself. What differs is the cost of starting processes, and whether the tool
keeps the file as it was.

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

1. **One media backend: libav\* in process, from Rust.** torcul calls
   libavformat, libavcodec, libavfilter and libavutil through the
   `ffmpeg-next` crate, and through `ffmpeg-sys-next` only for fields the safe
   API lacks; each `unsafe` block carries a `// SAFETY:` proof (HISS-09).
   This one backend covers probe, demux, mux, decode, filter and encode, in
   software (libdav1d, libsvtav1, libx265, and librav1e for issue #4) and in
   hardware (`*_qsv`, `*_vaapi`, `*_vulkan`), so the hardware choice stays a
   codec name, not a second framework. The "FFmpeg argument builder" crate
   of ADR-0001 decision 4 becomes an encoder settings crate that produces
   libavcodec options (an options dictionary per encoder), not command lines.
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
   binary: `plan`, `remux`, `verify`, `apply`) come first. Encode, hardware
   probe and scoring crates follow with issues #3 to #9. The Go service layer
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
   `Cargo.toml`), dav1d BSD-2-Clause, SVT-AV1 BSD-3-Clause-Clear with the
   Alliance for Open Media Patent License 1.0, x265 GPL-2.0-or-later
   (workstation package metadata and `/usr/share/licenses/svt-av1/`).

Rejected:

- **Direct codec libraries as the main path.** They measured no faster than
  libavcodec's wrappers for SVT-AV1 and x265, and only dav1d has a maintained
  Rust binding; the rest would be FFI torcul writes and maintains by hand,
  duplicating what libavcodec already maintains. Encoder options libavcodec
  does not name pass through `svtav1-params` and `x265-params`.
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

- **Positive**: one library for every stage and every codec, software or
  hardware; no process per file; remux output differs from the source only
  where the plan says so, and the verify stage proves it per packet; the
  crates fit the own-apps rule and can be reused by Aegis-OS (ADR-0001).
- **Negative**: torcul links FFmpeg, so a GPL FFmpeg build makes torcul
  binaries GPL-3.0 when distributed, and the system FFmpeg version becomes a
  build input (CI installs the FFmpeg development packages). `ffmpeg-next`
  is maintenance-only, so new FFmpeg APIs may reach torcul through
  `ffmpeg-sys-next` and `unsafe` first, or need a pull request upstream. Some
  libav\* behaviour has to be corrected in torcul, such as the cover image
  conversion in decision 3. Hardware choices stay unmeasured until the
  cluster measurement below lands.
- **Follow-up** (issue #11): measure hardware encode and decode on the
  cluster's Arc B60 and B580 (Quick Sync Video against VA-API against Vulkan Video, AV1 and
  HEVC, speed and vmafx score at equal size, and a decode, scale and encode
  chain that stays on the card); the result feeds codec tiering (issue #4)
  and the hardware probe (issue #5).
- **Revisit when**: an encoder control is needed that libavcodec cannot pass
  through; a direct library measures more than 10 % cheaper on cluster
  workloads; `ffmpeg-next` stops compiling against a current FFmpeg release;
  or live streaming becomes a goal, which is GStreamer's strength.

## References

- Order `torcul-kickoff-20261117`, operator direction of 2026-11-17
  (paraphrased): start the encode and decode project now; `ffmpeg`
  tool-based flows cost too much time. Tracking issue #10.
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
