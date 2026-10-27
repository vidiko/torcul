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

Bootstrap. The repository carries its standards set-up (praetor), licence and
first decision record; no service code yet.

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
