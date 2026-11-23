// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{
    Error, Result,
    error::{av, fs},
    io::{Deadline, open_regular},
    packets,
};
use ffmpeg_next::{self as ffmpeg, format, media};
use std::{collections::BTreeMap, path::Path};
use torcul_policy::{FileFacts, TrackInfo, TrackKind};

pub(crate) type Tags = BTreeMap<String, String>;
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Chapter {
    pub start_ns: i128,
    pub end_ns: i128,
    pub tags: Tags,
}
pub(crate) struct Snapshot {
    pub facts: FileFacts,
    pub packets: Vec<packets::PacketProof>,
    pub duration_us: Option<i64>,
    pub chapters: Vec<Chapter>,
    pub tags: Tags,
    pub stream_tags: Vec<Tags>,
    pub extra: Vec<String>,
}

/// Probe a local file; reject malformed/truncated packet streams.
pub fn probe(path: &Path) -> Result<FileFacts> {
    Ok(snapshot(path)?.facts)
}

pub(crate) fn open(path: &Path, deadline: Deadline) -> Result<format::context::Input> {
    fs("validate media path", open_regular(path, deadline))?;
    let canonical = fs("resolve media path", deadline.run(|| path.canonicalize()))?;
    let text = canonical
        .to_str()
        .filter(|s| !s.contains('\0'))
        .ok_or_else(|| Error::Invalid("media path must be UTF-8 without NUL".into()))?;
    av("initialize FFmpeg", ffmpeg::init())?;
    av(
        "open input with deadline",
        format::input_with_interrupt(&text, move || deadline.expired()),
    )
}

pub(crate) fn facts(input: &format::context::Input) -> Result<FileFacts> {
    if input.nb_streams() == 0 {
        return Err(Error::Invalid("media contains no streams".into()));
    }
    let tracks = input.streams().map(track).collect();
    Ok(FileFacts {
        tracks,
        container: input.format().name().to_owned(),
    })
}

fn track(stream: ffmpeg::Stream<'_>) -> TrackInfo {
    let kind = match stream.parameters().medium() {
        media::Type::Video => TrackKind::Video,
        media::Type::Audio => TrackKind::Audio,
        media::Type::Subtitle => TrackKind::Subtitle,
        media::Type::Attachment => TrackKind::Attachment,
        media::Type::Data => TrackKind::Data,
        _ => TrackKind::Other,
    };
    let dispositions = dispositions(&stream);
    TrackInfo {
        index: stream.index(),
        kind,
        codec: stream.parameters().id().name().to_owned(),
        language: stream.metadata().get("language").map(str::to_owned),
        title: stream.metadata().get("title").map(str::to_owned),
        attached_pic: dispositions.contains("attached_pic"),
        dispositions,
    }
}

fn dispositions(stream: &ffmpeg::Stream<'_>) -> std::collections::BTreeSet<String> {
    // SAFETY: stream owns a live AVStream; read-only access stays within its borrow.
    let bits = unsafe { (*stream.as_ptr()).disposition } as u32;
    (0..32)
        .filter(|bit| bits & (1_u32 << bit) != 0)
        .map(|bit| {
            // SAFETY: FFmpeg returns a static NUL-terminated name or NULL for any bit.
            let name = unsafe { ffmpeg::ffi::av_disposition_to_string((1_u32 << bit) as i32) };
            if name.is_null() {
                return format!("bit-{bit}");
            }
            // SAFETY: non-null name points to a static C string per av_disposition_to_string.
            unsafe { std::ffi::CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

pub(crate) fn snapshot(path: &Path) -> Result<Snapshot> {
    let deadline = Deadline::new();
    let mut input = open(path, deadline)?;
    let facts = facts(&input)?;
    let duration_us = (input.duration() != ffmpeg::ffi::AV_NOPTS_VALUE).then_some(input.duration());
    let chapters = input
        .chapters()
        .map(|c| Chapter {
            start_ns: nanos(c.start(), c.time_base()),
            end_ns: nanos(c.end(), c.time_base()),
            tags: tags(c.metadata()),
        })
        .collect();
    let stream_tags = input.streams().map(|s| tags(s.metadata())).collect();
    let extra = input
        .streams()
        .map(|s| packets::extradata_hash(&s))
        .collect();
    let tags = tags(input.metadata());
    let packets = packets::proofs(&mut input, deadline)?;
    packets::validate_presence(&facts, &packets)?;
    Ok(Snapshot {
        facts,
        packets,
        duration_us,
        chapters,
        tags,
        stream_tags,
        extra,
    })
}

fn nanos(value: i64, base: ffmpeg::Rational) -> i128 {
    i128::from(value) * i128::from(base.numerator()) * 1_000_000_000
        / i128::from(base.denominator())
}
pub(crate) fn tags(dictionary: ffmpeg::DictionaryRef<'_>) -> Tags {
    dictionary
        .iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}
