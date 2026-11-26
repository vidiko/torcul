---
description: Vidiko’s video encode service and its track-policy command-line tool.
---

# torcul

torcul is Vidiko’s video encode service. Its planned encode job takes a media
file and a stream index. It returns a verified video-only file with a record
of how it was encoded, or a verdict to keep the original.
[Encode service design](adr/0001-vmafx-integration.md)

**Available today.** The Rust CLI plans which tracks to keep, copies them into
a staging directory and verifies the copy against the source. The library
file stays in place. Encoding, language checks and applying a verified change
are planned. [Track-policy CLI](guides/track-policy-cli.md) ·
[ADR-0002, first slice](adr/0002-pipeline-architecture.md#decision-17-first-slice)

## The name

Pronounce *torcul* **TOR-kul** (/ˈtɔr.kʊl/). The name shortens the Latin
*torcular*, a wine or oil press, from *torquere*, “to twist”. As a press takes
in fruit and yields a pressed product, the encode service takes in a video
stream and aims to produce a smaller encode of that same stream.
[Project naming note](../README.md) · [Wiktionary, *torcular*](https://en.wiktionary.org/wiki/torcular#Latin)

## Where to go next

- [Track-policy CLI](guides/track-policy-cli.md) walks through a dry run, a
  remux and verification using generated media.
- [Architecture decisions](adr/README.md) explain how torcul selects media
  backends and where VMAFx fits.
- [Repository](../) contains the Rust workspace, fixtures and contribution
  guidelines.
