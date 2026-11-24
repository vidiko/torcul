# torcul

torcul is the video encode service of the Vidiko organisation. It takes one
video stream from a media file and returns a verified encode of that stream,
or the verdict that the original should be kept.

## The name

- **Meaning**: a press, as in a wine or oil press. Input goes in, a pressed
  product comes out; here a source video stream goes in and a smaller encode
  of the same stream comes out.
- **Origin**: Latin *torcular* (press, wine press), from *torquere* (to
  twist), shortened to *torcul*.
- **Pronunciation**: TOR-kul (/ˈtɔr.kʊl/).

## What it does

- One video stream in (a media file plus a stream index), one video-only
  file plus a provenance record out.
- Auto mode first: torcul picks the codec, the encoder and the rate control
  from a quality target and a size budget, and keeps the original when no
  encode meets both.
- Hardware encoders are probed with a test encode; remux, CPU encode and GPU
  encode run in bounded pools.
- Geometry (crop, aspect ratio, frame rate, deinterlace) and HDR metadata
  (HDR10, HDR10+, Dolby Vision) are carried through.

The encode job contract started as ADR-3378 in
[VMAFx/vmafx](https://github.com/VMAFx/vmafx) and moved here on 2026-10-27.
The work items are tracked as issues in this repository.

## Quality: torcul uses VMAFx

torcul does not measure quality itself. It connects to
[VMAFx](https://github.com/VMAFx/vmafx), the perceptual video quality library
and toolkit, for:

- **scoring**: VMAF scores of each candidate encode against its source;
- **`vmaf-tune auto`**: the quality search that picks encoder settings for a
  target score;
- **compare**: codec and encoder comparison on the same source.

How torcul binds VMAFx (in-process over the `libvmafx` C interface, through
Rust crates) is recorded in
[ADR-0001](docs/adr/0001-vmafx-integration.md).

## Status

The Rust workspace implements the first track-policy remux slice
([ADR-0002](docs/adr/0002-pipeline-architecture.md) decision 17): dry-run
planning, stream copy into staging, and lossless verification. Library files
remain untouched until the language-check and apply slice lands. Encoding
services described above remain planned; this slice never re-encodes.

### Track-policy CLI

Install the Rust toolchain declared in [rust-toolchain.toml](rust-toolchain.toml),
FFmpeg development libraries (libavformat, libavcodec, libavutil), the FFmpeg CLI,
Clang, pkg-config and Python 3, then build:

```sh
cargo build --locked
python3 -I tools/fixtures/track_fixtures.py target/fx
target/debug/torcul plan --meta target/fx/manifest.json --json target/plan.json target/fx
target/debug/torcul remux --meta target/fx/manifest.json --staging target/staging target/fx/f07_default_dropped.mkv
target/debug/torcul verify --staging target/staging target/fx/f07_default_dropped.mkv
```

Metadata is JSON with a `fixtures` array: each entry needs `name` (basename),
`original` (ISO 639 language code or null), and `anime` (boolean). Extra keys
are ignored. Missing entries or invalid ISO 639 `original` codes produce
per-file errors and a nonzero exit status. Planning a directory scans its direct
children with recognized media extensions; it does not descend into directories.
Tracks dropped by the policy carry `"lid": "pending"` (`drop (pending LID)` in
the human table) until the language-check stage exists.

The policy keeps English/German audio, original audio when neither is present,
and all three for anime (retaining every audio track with a warning when anime
original language is null). Unknown-language audio/subtitles remain with
warnings; no matching audio falls back to retaining all audio. English/German
subtitles remain in every variant. Video, cover art, attachments, chapters and
global metadata remain. See [policy implementation](crates/torcul-policy/src/lib.rs)
and [fixture oracle](tools/fixtures/track_fixtures.py).

Matroska and MP4 remuxes copy packets and codec parameters; remux never encodes.
Verification ([ADR-0002](docs/adr/0002-pipeline-architecture.md) decisions 4
and 13) maps source streams explicitly to staged streams and compares per-kind
stream counts, chapter counts and exact rational chapter timestamps, attachment
counts and payloads, per-packet payload size, key flag, timestamps (`pts`,
`dts`, `duration` within one tick of the coarser stream time base), payload
SHA-256, codec parameters with extradata, decode/display side data, and
bidirectional tag maps (allowing muxer-written Matroska tags; see
[verification](crates/torcul-media/src/verify.rs)).
Some fragmented MP4 subtitle layouts change packet framing in FFmpeg's muxer;
verification refuses those outputs. Original encoder tags remain. MP4 preserves
opaque movie/track metadata boxes with
[mp4-atom](https://docs.rs/mp4-atom/0.17.1/mp4_atom/); file-type changes adjust
sample chunk offsets without changing packet bytes. Metadata boxes have a
64 MiB bound; larger boxes fail with an error. Language normalization uses
[rust_iso639](https://docs.rs/rust_iso639/0.1.0/rust_iso639/), and media I/O
uses [ffmpeg-next](https://docs.rs/ffmpeg-next/9.1.0/ffmpeg_next/).

Staging writes `<name>.torcul.json` beside the staged file with the plan and
the source fingerprint (`dev`, `ino`, `size`, `mtime_ns`, and whole-file
`sha256:<hex>` verified by matching `fstat` before and after hashing). Sources
with multiple hard links and staging directories on a different filesystem
device are refused. Verification records its verdict and the full fingerprint
of the staged output there. Only files needing remux create staging output.
Existing staging files are refused. Input paths must be regular local files
with UTF-8 names; symlinks/devices/FIFOs are refused.

FFmpeg reads use interrupt deadlines; packet loops also have a scalar limit.
Local file operations check deadlines around syscalls, but a kernel stall
inside a regular-file syscall cannot be cancelled portably. No concurrent
writer should modify sources, staging, or sidecars during these commands.

## Development

```sh
make verify-all
```

runs the same gate as CI. Contribution rules are in
[CONTRIBUTING.md](CONTRIBUTING.md), agent rules in [AGENTS.md](AGENTS.md),
security reports in [SECURITY.md](SECURITY.md).

## Licence

torcul is licensed under the European Union Public Licence 1.2
([LICENSE](LICENSE)). Licence and copyright information for every file
follows the [REUSE](https://reuse.software) specification
([REUSE.toml](REUSE.toml), [LICENSES/](LICENSES/)).

<!-- praetor:readme-governance:start -->
[![HISS Adopted][praetor-hiss-badge]][praetor-hiss-agents]

Praetor manages this repository's declared governance policy. This managed
block records adoption state; it is not a verification certificate.

**Verification**: `make verify-all` runs the repository's configured
verification cascade.

**HISS Audit**: `praetorctl audit` enforces policy, generated-surface
integrity, and the debt ratchet.

**Context Sync**: `praetorctl compile-context --verify` verifies every
generated agent context against `AGENTS.md`.

**Debt Baseline**: `.standards-baseline.json` anchors the debt ratchet at
0 recorded infractions; audit forbids growth.

[praetor-hiss-badge]: https://img.shields.io/badge/Standards-HISS%20Adopted-blue
[praetor-hiss-agents]: https://github.com/vidiko/torcul/blob/HEAD/AGENTS.md
<!-- praetor:readme-governance:end -->
