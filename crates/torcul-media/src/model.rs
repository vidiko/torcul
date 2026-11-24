// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use torcul_policy::{Plan, TrackInfo, TrackKind};

pub type Tags = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeBase {
    pub num: i32,
    pub den: i32,
}

impl TimeBase {
    pub fn normalized(num: i32, den: i32) -> Self {
        if num == 0 || den == 0 {
            return Self { num: 0, den: 1 };
        }
        let num64 = i64::from(num);
        let den64 = i64::from(den);
        let divisor = i64::from(gcd(num.unsigned_abs(), den.unsigned_abs()));
        let sign = if den64 < 0 { -1_i64 } else { 1_i64 };
        let reduced_num = sign * (num64 / divisor);
        let reduced_den = sign * (den64 / divisor);
        match (i32::try_from(reduced_num), i32::try_from(reduced_den)) {
            (Ok(n), Ok(d)) => Self { num: n, den: d },
            _ => Self { num: 0, den: 1 },
        }
    }

    pub fn coarser(self, other: Self) -> Self {
        let lhs = i128::from(self.num) * i128::from(other.den);
        let rhs = i128::from(other.num) * i128::from(self.den);
        if lhs >= rhs { self } else { other }
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    for _ in 0..64 {
        if b == 0 {
            break;
        }
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a.max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SideDataKind {
    DisplayMatrix,
    MasteringDisplayMetadata,
    ContentLightLevel,
    DoviConf,
    SkipSamples,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SideDataRecord {
    pub kind: SideDataKind,
    pub data: Box<[u8]>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelLayout {
    pub order: i32,
    pub nb_channels: i32,
    pub mask: u64,
    pub custom_ids: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecParams {
    pub codec_id: u32,
    pub profile: i32,
    pub level: i32,
    pub width: i32,
    pub height: i32,
    pub format: i32,
    pub frame_rate: TimeBase,
    pub sample_rate: i32,
    pub channel_layout: ChannelLayout,
    pub extradata: Vec<u8>,
    pub coded_side_data: Vec<SideDataRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketRecord {
    pub size: u64,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub duration: Option<i64>,
    pub key: bool,
    pub side_data: Box<[SideDataRecord]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketProof {
    pub count: u64,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChapterRecord {
    pub id: i64,
    pub time_base: TimeBase,
    pub start: i64,
    pub end: i64,
    pub tags: Tags,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamSnapshot {
    pub track: TrackInfo,
    pub time_base: TimeBase,
    pub codec_params: CodecParams,
    pub tags: Tags,
    pub proof: PacketProof,
    pub packets: Vec<PacketRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSnapshot {
    pub container: String,
    pub streams: Vec<StreamSnapshot>,
    pub chapters: Vec<ChapterRecord>,
    pub tags: Tags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamMapping {
    pub source: usize,
    pub staging: usize,
    pub attached_pic: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamCounts {
    pub video: usize,
    pub audio: usize,
    pub subtitle: usize,
    pub attachment: usize,
}

impl StreamCounts {
    pub fn add(&mut self, kind: TrackKind) {
        match kind {
            TrackKind::Video => self.video += 1,
            TrackKind::Audio => self.audio += 1,
            TrackKind::Subtitle => self.subtitle += 1,
            TrackKind::Attachment => self.attachment += 1,
            TrackKind::Data | TrackKind::Other => {}
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaCounts {
    pub streams: StreamCounts,
    pub chapters: usize,
    pub attachments: usize,
}

impl MediaSnapshot {
    pub fn counts(&self) -> MediaCounts {
        let mut streams = StreamCounts::default();
        for stream in &self.streams {
            streams.add(stream.track.kind);
        }
        let attachments = streams.attachment;
        MediaCounts {
            streams,
            chapters: self.chapters.len(),
            attachments,
        }
    }
}

pub fn expected_counts(source: &MediaSnapshot, plan: &Plan) -> MediaCounts {
    let mut streams = StreamCounts::default();
    for index in plan.kept_indices() {
        if let Some(stream) = source.streams.get(index) {
            streams.add(stream.track.kind);
        }
    }
    let attachments = streams.attachment;
    MediaCounts {
        streams,
        chapters: source.chapters.len(),
        attachments,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub pass: bool,
    pub failures: Vec<String>,
    pub stream_map: Vec<StreamMapping>,
    pub source_counts: MediaCounts,
    pub output_counts: MediaCounts,
    pub source_packets: Vec<PacketProof>,
    pub staging_packets: Vec<PacketProof>,
}

impl VerifyReport {
    pub fn failed(message: String) -> Self {
        Self {
            pass: false,
            failures: vec![message],
            stream_map: Vec::new(),
            source_counts: MediaCounts::default(),
            output_counts: MediaCounts::default(),
            source_packets: Vec::new(),
            staging_packets: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TimeBase;

    #[test]
    fn time_base_normalized_handles_min_extremes_and_signs() {
        assert_eq!(
            TimeBase::normalized(i32::MIN, i32::MIN),
            TimeBase { num: 1, den: 1 }
        );
        assert_eq!(
            TimeBase::normalized(i32::MIN, -1),
            TimeBase { num: 0, den: 1 }
        );
        assert_eq!(
            TimeBase::normalized(1, i32::MIN),
            TimeBase { num: 0, den: 1 }
        );
        assert_eq!(TimeBase::normalized(6, -4), TimeBase { num: -3, den: 2 });
        assert_eq!(TimeBase::normalized(0, 5), TimeBase { num: 0, den: 1 });
        assert_eq!(TimeBase::normalized(5, 0), TimeBase { num: 0, den: 1 });
        assert_eq!(
            TimeBase::normalized(i32::MAX, 1),
            TimeBase {
                num: i32::MAX,
                den: 1
            }
        );
        assert_eq!(
            TimeBase::normalized(i32::MIN, 1),
            TimeBase {
                num: i32::MIN,
                den: 1
            }
        );
        assert_eq!(
            TimeBase::normalized(i32::MIN, -2),
            TimeBase {
                num: 1_073_741_824,
                den: 1
            }
        );
    }
}
