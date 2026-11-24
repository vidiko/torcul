// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{
    Error, Result,
    error::{av, fs},
    io::{Deadline, open_regular},
    model::{ChapterRecord, MediaSnapshot, StreamSnapshot, Tags, TimeBase},
    packets,
};
use ffmpeg_next::{self as ffmpeg, format, media};
use std::path::Path;
use torcul_policy::{FileFacts, TrackInfo, TrackKind};

/// Probe a local file; reject malformed/truncated packet streams.
pub fn probe(path: &Path) -> Result<FileFacts> {
    let deadline = Deadline::new();
    let mut input = open(path, deadline)?;
    let facts = facts(&input)?;
    let proofs = packets::proofs(&mut input, deadline)?;
    packets::validate_presence(&facts, &proofs)?;
    Ok(facts)
}

/// Read complete stream, chapter, metadata, codec-parameter and packet snapshots.
pub fn snapshot(path: &Path) -> Result<MediaSnapshot> {
    let deadline = Deadline::new();
    let mut input = open(path, deadline)?;
    let facts = facts(&input)?;
    let chapters = input
        .chapters()
        .map(|c| ChapterRecord {
            id: c.id(),
            time_base: rational_base(c.time_base()),
            start: c.start(),
            end: c.end(),
            tags: tags(c.metadata()),
        })
        .collect();
    let stream_meta: Vec<_> = input
        .streams()
        .map(|s| {
            (
                rational_base(s.time_base()),
                packets::codec_params(&s),
                tags(s.metadata()),
            )
        })
        .collect();
    let container_tags = tags(input.metadata());
    let collected = packets::collect_packets(&mut input, deadline)?;
    let proofs: Vec<_> = collected.iter().map(|(proof, _)| proof.clone()).collect();
    packets::validate_presence(&facts, &proofs)?;
    let streams = facts
        .tracks
        .into_iter()
        .zip(stream_meta)
        .zip(collected)
        .map(
            |((track, (time_base, codec_params, stream_tags)), (proof, records))| StreamSnapshot {
                track,
                time_base,
                codec_params,
                tags: stream_tags,
                proof,
                packets: records,
            },
        )
        .collect();
    Ok(MediaSnapshot {
        container: facts.container,
        streams,
        chapters,
        tags: container_tags,
    })
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
        chapters: input.nb_chapters() as usize,
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
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI read of AVStream.disposition, which ffmpeg-next 9.1.0 truncates to known bits; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next exposes every disposition bit
    let bits = unsafe { (*stream.as_ptr()).disposition } as u32;
    (0..32)
        .filter(|bit| bits & (1_u32 << bit) != 0)
        .map(|bit| {
            // SAFETY: FFmpeg returns a static NUL-terminated name or NULL for any bit.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI call to av_disposition_to_string, which has no ffmpeg-next 9.1.0 wrapper; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next wraps av_disposition_to_string
            let name = unsafe { ffmpeg::ffi::av_disposition_to_string((1_u32 << bit) as i32) };
            if name.is_null() {
                return format!("bit-{bit}");
            }
            // SAFETY: non-null name points to a static C string per av_disposition_to_string.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- CStr borrow of static C string from av_disposition_to_string, which has no ffmpeg-next 9.1.0 wrapper; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next wraps av_disposition_to_string
            unsafe { std::ffi::CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn rational_base(base: ffmpeg::Rational) -> TimeBase {
    TimeBase::normalized(base.numerator(), base.denominator())
}

pub(crate) fn tags(dictionary: ffmpeg::DictionaryRef<'_>) -> Tags {
    dictionary
        .iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}
