// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod error;
mod inspect;
pub mod io;
mod model;
mod mp4;
mod mp4_boxes;
mod mp4_movie;
mod packets;
mod remux;
mod verify;

pub use error::{Error, Result};
pub use inspect::{probe, snapshot};
pub use model::{
    ChannelLayout, ChapterRecord, CodecParams, MediaCounts, MediaSnapshot, PacketProof,
    PacketRecord, SideDataKind, SideDataRecord, StreamCounts, StreamMapping, StreamSnapshot,
    TimeBase, VerifyReport, expected_counts,
};
pub use remux::{RemuxStats, remux};
pub use verify::{
    MUXER_WRITTEN_TAGS, build_stream_map, compare_chapters, compare_codec_params,
    compare_expected_counts, compare_packets, compare_side_data, compare_tags, verify,
    verify_snapshots,
};
