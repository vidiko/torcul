// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{Error, Result, io::Deadline};
use ffmpeg_next::{self as ffmpeg, Packet, format::context::Input};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use torcul_policy::{FileFacts, TrackKind};

pub(crate) const MAX_PACKETS: u64 = 1_000_000_000;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PacketProof {
    pub count: u64,
    pub bytes: u64,
    pub sha256: String,
}

/// Read directly: FFmpeg's convenience iterator suppresses some read errors.
pub(crate) fn read(input: &mut Input, deadline: Deadline) -> Result<Option<Packet>> {
    crate::error::fs("packet deadline", deadline.check())?;
    let mut packet = Packet::empty();
    match packet.read(input) {
        Ok(()) if packet.is_corrupt() => Err(Error::Invalid("corrupt packet".into())),
        Ok(()) => Ok(Some(packet)),
        Err(ffmpeg::Error::Eof) => Ok(None),
        Err(source) => Err(Error::Ffmpeg {
            operation: "read packet",
            source,
        }),
    }
}

pub(crate) fn proofs(input: &mut Input, deadline: Deadline) -> Result<Vec<PacketProof>> {
    let mut hashes = vec![Sha256::new(); input.nb_streams() as usize];
    let mut counts = vec![0_u64; hashes.len()];
    let mut sizes = vec![0_u64; hashes.len()];
    for _ in 0..MAX_PACKETS {
        let Some(packet) = read(input, deadline)? else {
            return Ok(hashes
                .into_iter()
                .enumerate()
                .map(|(i, h)| PacketProof {
                    count: counts[i],
                    bytes: sizes[i],
                    sha256: crate::io::hex(&h.finalize()),
                })
                .collect());
        };
        let i = packet.stream();
        let hash = hashes
            .get_mut(i)
            .ok_or_else(|| Error::Invalid("packet stream out of range".into()))?;
        counts[i] += 1;
        sizes[i] += packet.size() as u64;
        hash.update(packet.data().unwrap_or_default());
    }
    Err(Error::Invalid(format!(
        "packet bound {MAX_PACKETS} exceeded"
    )))
}

pub(crate) fn validate_presence(facts: &FileFacts, proofs: &[PacketProof]) -> Result<()> {
    for (track, proof) in facts.tracks.iter().zip(proofs) {
        if matches!(track.kind, TrackKind::Audio | TrackKind::Video) && proof.count == 0 {
            return Err(Error::Invalid(format!(
                "stream {} contains no packets (possibly truncated)",
                track.index
            )));
        }
    }
    Ok(())
}

pub(crate) fn extradata_hash(stream: &ffmpeg::Stream<'_>) -> String {
    // SAFETY: codec parameters and extradata remain live for stream's borrow;
    // FFmpeg guarantees extradata_size bytes at extradata when size is positive.
    let bytes = unsafe {
        let parameters = stream.parameters();
        let raw = &*parameters.as_ptr();
        if raw.extradata.is_null() || raw.extradata_size <= 0 {
            &[]
        } else {
            std::slice::from_raw_parts(raw.extradata, raw.extradata_size as usize)
        }
    };
    crate::io::hex(&Sha256::digest(bytes))
}

pub(crate) fn picture_attachment(
    source: &ffmpeg::Stream<'_>,
    target: &mut ffmpeg::StreamMut<'_>,
) -> Result<()> {
    // AVStream.attached_pic is demux-only (libavformat/avformat.h).
    // Matroska represents covers as attachments whose payload is extradata.
    // SAFETY: both streams live; target exclusively borrowed. New padded buffer
    // allocated with FFmpeg allocator, transferred to target codecpar ownership;
    // old extradata freed with its matching allocator. No pointers escape.
    unsafe {
        let packet = &(*source.as_ptr()).attached_pic;
        if packet.size <= 0 || packet.data.is_null() {
            return Err(Error::Invalid("attached picture has no payload".into()));
        }
        let size = packet.size as usize;
        let data =
            ffmpeg::ffi::av_mallocz(size + ffmpeg::ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize)
                .cast::<u8>();
        if data.is_null() {
            return Err(Error::Invalid("allocate cover attachment payload".into()));
        }
        std::ptr::copy_nonoverlapping(packet.data, data, size);
        let parameters = &mut *(*target.as_mut_ptr()).codecpar;
        ffmpeg::ffi::av_freep((&raw mut parameters.extradata).cast());
        parameters.extradata = data;
        parameters.extradata_size = packet.size;
        parameters.codec_type = ffmpeg::ffi::AVMediaType::AVMEDIA_TYPE_ATTACHMENT;
    }
    Ok(())
}
