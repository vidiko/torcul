# Architectural Decision Records (ADRs)

This directory documents key architectural decisions. Records are append-only
like public APIs under HISS-14 (Append-Only ABI & Migration Footers).
An accepted record keeps its number and text; a changed decision is a new
record that supersedes it. New decisions that leave the accepted ones as they
are may be appended to a record as a dated extension section with its own
context, consequences and references; the status line and the table below
then name the extension.

| Number | Date | Title | Status |
| :--- | :--- | :--- | :--- |
| [0000](0000-template.md) | 2026-09-11 | ADR template | Accepted |
| [0001](0001-vmafx-integration.md) | 2026-10-27 | torcul uses vmafx in process through the libvmafx C ABI | Accepted; decision 4 in part superseded by 0002 |
| [0002](0002-pipeline-architecture.md) | 2026-11-17 | torcul runs its media pipeline in process from Rust, through libav\* and direct SVT-AV1 | Accepted; extended 2026-11-23 by decisions 8 to 17 (execution ownership, apply and rollback) |
