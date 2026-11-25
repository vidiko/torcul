// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
//! Probe, remux into staging, and verify media files through FFmpeg, with a source I/O layer.
//!
//! The default `ffmpeg` feature links FFmpeg through `ffmpeg-next` and enables `probe`,
//! `snapshot`, `remux`, `verify`, `verify_snapshots`, the `compare_*` functions, and model types.
//! When the resolved build has the `ffmpeg` feature off (for example, with
//! `default-features = false` and no other package enabling it; Cargo unifies features), only the
//! `io` module, `Error`, and `Result` remain, and `Error` has no `Ffmpeg` or `Atom` variant. This
//! build needs no FFmpeg development files and is used by the Windows CI job. A consumer that
//! turns default features off and calls these items should declare `features = ["ffmpeg"]`, so
//! they stay available when no other package in the build enables the feature.

mod error;
#[cfg(feature = "ffmpeg")]
mod inspect;
pub mod io;
#[cfg(feature = "ffmpeg")]
mod model;
#[cfg(feature = "ffmpeg")]
mod mp4;
#[cfg(feature = "ffmpeg")]
mod mp4_boxes;
#[cfg(feature = "ffmpeg")]
mod mp4_movie;
#[cfg(feature = "ffmpeg")]
mod packets;
#[cfg(feature = "ffmpeg")]
mod remux;
#[cfg(feature = "ffmpeg")]
mod verify;

pub use error::{Error, Result};
#[cfg(feature = "ffmpeg")]
pub use inspect::{probe, snapshot};
#[cfg(feature = "ffmpeg")]
pub use model::{
    ChannelLayout, ChapterRecord, CodecParams, MediaCounts, MediaSnapshot, PacketProof,
    PacketRecord, SideDataKind, SideDataRecord, StreamCounts, StreamMapping, StreamSnapshot,
    TimeBase, VerifyReport, expected_counts,
};
#[cfg(feature = "ffmpeg")]
pub use remux::{RemuxStats, remux};
#[cfg(feature = "ffmpeg")]
pub use verify::{
    MUXER_WRITTEN_TAGS, build_stream_map, compare_chapters, compare_codec_params,
    compare_expected_counts, compare_packets, compare_side_data, compare_tags, verify,
    verify_snapshots,
};
