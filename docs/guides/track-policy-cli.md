---
description: Build torcul, plan track selection and verify a staged remux against its source.
---

# Track-policy CLI

Use `plan` to see which tracks torcul would keep. Use `remux` to copy the
selected tracks into staging, then `verify` to check them against the source.
All three commands leave the library file in place. Encoding, language checks
and applying the result are planned.
[CLI commands][cli] · [ADR-0002, decision 17][first-slice]

## Build and try a dry run

On Unix, install the [pinned Rust toolchain][rust], Clang, pkg-config and the
FFmpeg development libraries libavformat, libavcodec and libavutil. The
[fixture generator][fixtures] also needs Python 3 and the FFmpeg CLI with
libx264. The [media crate][media-crate] enables its `ffmpeg` feature by
default. [Windows CI][ci] builds that crate without the feature and runs its
non-Unix I/O tests.

From the repository root, build the CLI and generate sample media and a
metadata manifest. Plan the sample directory and save the report as JSON.

```sh
cargo build --locked
python3 -I tools/fixtures/track_fixtures.py target/fx
target/debug/torcul plan --meta target/fx/manifest.json \
  --json target/plan.json target/fx
```

Read the table to see the input indices kept and dropped for each file,
whether it needs a remux, and any warnings. A drop appears as
`drop (pending LID)` in the table and `"lid": "pending"` in JSON until the
language-check stage exists. [Plan report][report] · [Track policy][policy] ·
[JSON Schemas][schemas]

## Metadata and track selection

Pass a JSON manifest with a `fixtures` array to `--meta`. Supply one entry
per input with its `name` (basename), `original` (ISO 639 language code or
`null`) and `anime` (boolean). Extra keys are ignored. A missing entry or
invalid original-language code produces a per-file error and a nonzero exit
status. For a directory, `plan` considers only direct children with recognised
media extensions. [Manifest reader][report] · [Directory scan][files] ·
[Language normalisation with rust_iso639][language]

- **Audio.** Keep English and German. If neither is present, keep the original
  language. For anime, keep all three. If the original language of an anime
  title is unknown, keep every audio track and warn. If no audio matches the
  policy, keep all audio.
- **Subtitles.** Keep English and German for every title. Keep audio and
  subtitle tracks of undetermined language and warn.
- **Other content.** Keep video, cover art, attachments, chapters and global
  metadata.

The [policy rules][policy] and the [fixture generator][fixtures] give the
expected selection for each example.

## Stage and verify a remux

Use the same source path for both commands. The sample below needs a remux.
Keep the source, staging file and sidecar free of concurrent writes throughout.
[Staging and verification][staging]

```sh
target/debug/torcul remux --meta target/fx/manifest.json \
  --staging target/staging target/fx/f07_default_dropped.mkv
target/debug/torcul verify --staging target/staging \
  target/fx/f07_default_dropped.mkv
```

For Matroska and MP4, torcul copies packets and codec parameters through
ffmpeg-next. Original encoder tags survive. MP4 uses mp4-atom to preserve
opaque movie and track metadata boxes. A change in the file-type box adjusts
sample chunk offsets while preserving packet bytes. Metadata boxes larger
than 64 MiB produce an error. [Remux][remux] · [MP4 metadata][mp4] ·
[Box limits][boxes] · [Crate dependencies][media-crate]

Only inputs that need a remux produce staging output. Use a staging directory
on the same filesystem device as the source. torcul refuses existing staging
files and sources with multiple hard links. Input paths must name regular
local files with UTF-8 names. Symlinks, devices and FIFOs are refused.
[Staging rules][staging] · [Local file handling][io]

Beside the staged file, `<name>.torcul.json` records the plan and the source
fingerprint. It includes `dev`, `ino`, `size`, `mtime_ns` and a whole-file
`sha256:<hex>` digest. The file’s `fstat` values must match before and after
hashing. Verification writes its verdict to the sidecar and, on success, the
full fingerprint of the staged file. [Fingerprint][io] · [Sidecar][staging]

## What verification compares

The [verifier][verify] maps each kept source stream to its staged stream.
[ADR-0002, decisions 4 and 13][verification] define what must match.

- Stream counts by kind, chapter counts and exact rational chapter
  timestamps, and attachment counts and payloads.
- Each stream’s SHA-256 digest over its packet payloads, plus the size and
  key flag of each packet. Packet timestamps
  (`pts`, `dts`, `duration`) may differ by at most one tick of the coarser
  stream time base.
- Codec parameters including extradata, and side data used for decoding
  and display.
- Tag maps in both directions, allowing the Matroska tags the muxer writes.

Some fragmented MP4 subtitle layouts change packet framing in FFmpeg’s muxer.
Verification rejects the result when the packet comparison fails.
[Fragmented subtitle fixture][round-trip]

## I/O deadlines

FFmpeg reads use interrupt deadlines. Packet loops also bound the number of
packets. Local file operations check a deadline before and after each system
call. A kernel stall inside a regular-file call can still outlast the deadline.
[FFmpeg input][inspect] · [Packet limits][packets] · [Local I/O deadlines][io]

[cli]: ../../crates/torcul-cli/src/args.rs
[first-slice]: ../adr/0002-pipeline-architecture.md#decision-17-first-slice
[rust]: ../../rust-toolchain.toml
[fixtures]: ../../tools/fixtures/track_fixtures.py
[media-crate]: ../../crates/torcul-media/Cargo.toml
[ci]: ../../.github/workflows/ci.yml
[report]: ../../crates/torcul-cli/src/report.rs
[policy]: ../../crates/torcul-policy/src/lib.rs
[files]: ../../crates/torcul-cli/src/files.rs
[language]: ../../crates/torcul-policy/src/language.rs
[staging]: ../../crates/torcul-cli/src/staging.rs
[remux]: ../../crates/torcul-media/src/remux.rs
[mp4]: ../../crates/torcul-media/src/mp4.rs
[boxes]: ../../crates/torcul-media/src/mp4_boxes.rs
[io]: ../../crates/torcul-media/src/io.rs
[verify]: ../../crates/torcul-media/src/verify.rs
[verification]: ../adr/0002-pipeline-architecture.md#decision-13-verification
[round-trip]: ../../crates/torcul-media/tests/metadata_round_trip.rs
[inspect]: ../../crates/torcul-media/src/inspect.rs
[packets]: ../../crates/torcul-media/src/packets.rs
[schemas]: ../reference/json-schemas.md
