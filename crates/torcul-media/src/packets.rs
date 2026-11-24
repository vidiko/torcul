// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{
    Error, Result,
    io::Deadline,
    model::{
        ChannelLayout, CodecParams, PacketProof, PacketRecord, SideDataKind, SideDataRecord,
        TimeBase,
    },
};
use ffmpeg_next::{self as ffmpeg, Packet, format::context::Input, packet::Ref};
use sha2::{Digest, Sha256};
use torcul_policy::{FileFacts, TrackKind};

pub(crate) const MAX_PACKETS: u64 = 1_000_000_000;

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
    let stream_count = input.nb_streams() as usize;
    let mut hashes = vec![Sha256::new(); stream_count];
    let mut counts = vec![0_u64; stream_count];
    let mut sizes = vec![0_u64; stream_count];
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
        let index = packet.stream();
        let hash = hashes
            .get_mut(index)
            .ok_or_else(|| Error::Invalid("packet stream out of range".into()))?;
        counts[index] += 1;
        sizes[index] += packet.size() as u64;
        hash.update(packet.data().unwrap_or_default());
    }
    Err(Error::Invalid(format!(
        "packet bound {MAX_PACKETS} exceeded"
    )))
}

pub(crate) fn collect_packets(
    input: &mut Input,
    deadline: Deadline,
) -> Result<Vec<(PacketProof, Vec<PacketRecord>)>> {
    let stream_count = input.nb_streams() as usize;
    let mut hashes = vec![Sha256::new(); stream_count];
    let mut counts = vec![0_u64; stream_count];
    let mut sizes = vec![0_u64; stream_count];
    let mut records: Vec<Vec<PacketRecord>> = (0..stream_count).map(|_| Vec::new()).collect();
    for _ in 0..MAX_PACKETS {
        let Some(packet) = read(input, deadline)? else {
            return Ok(hashes
                .into_iter()
                .zip(records)
                .enumerate()
                .map(|(i, (h, stream_records))| {
                    (
                        PacketProof {
                            count: counts[i],
                            bytes: sizes[i],
                            sha256: crate::io::hex(&h.finalize()),
                        },
                        stream_records,
                    )
                })
                .collect());
        };
        let index = packet.stream();
        let hash = hashes
            .get_mut(index)
            .ok_or_else(|| Error::Invalid("packet stream out of range".into()))?;
        counts[index] += 1;
        sizes[index] += packet.size() as u64;
        hash.update(packet.data().unwrap_or_default());
        records[index].push(packet_record(&packet));
    }
    Err(Error::Invalid(format!(
        "packet bound {MAX_PACKETS} exceeded"
    )))
}

fn packet_record(packet: &Packet) -> PacketRecord {
    let raw_duration = packet.duration();
    let duration = (raw_duration != ffmpeg::ffi::AV_NOPTS_VALUE).then_some(raw_duration);
    // SAFETY: packet owns a live AVPacket; side_data points to side_data_elems entries when non-null.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI read of AVPacket.side_data, because ffmpeg-next 9.1.0 SideData::data() skips null checks and SideData::kind() can hit unimplemented!(); proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next soundly wraps packet side data
    let side_data = unsafe {
        let raw = &*packet.as_ptr();
        raw_side_data(raw.side_data, raw.side_data_elems)
    };
    PacketRecord {
        size: packet.size() as u64,
        pts: packet.pts(),
        dts: packet.dts(),
        duration,
        key: packet.is_key(),
        side_data: side_data.into_boxed_slice(),
    }
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

pub(crate) fn codec_params(stream: &ffmpeg::Stream<'_>) -> CodecParams {
    let parameters = stream.parameters();
    let avg_rate = stream.avg_frame_rate();
    // SAFETY: parameters holds a live AVCodecParameters tied to stream's borrow;
    // extradata, coded_side_data and custom channel maps remain valid for their positive counts.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI read of AVCodecParameters fields, because ffmpeg-next 9.1.0 codec::Parameters exposes only medium, id and bit_rate; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next exposes full codec parameters and coded side data
    unsafe {
        let raw = &*parameters.as_ptr();
        let extradata = if raw.extradata.is_null() || raw.extradata_size <= 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(raw.extradata, raw.extradata_size as usize).to_vec()
        };
        let frame_rate = if raw.framerate.num != 0 && raw.framerate.den != 0 {
            TimeBase::normalized(raw.framerate.num, raw.framerate.den)
        } else {
            TimeBase::normalized(avg_rate.numerator(), avg_rate.denominator())
        };
        CodecParams {
            codec_id: raw.codec_id as u32,
            profile: raw.profile,
            level: raw.level,
            width: raw.width,
            height: raw.height,
            format: raw.format,
            frame_rate,
            sample_rate: raw.sample_rate,
            channel_layout: channel_layout(&raw.ch_layout),
            extradata,
            coded_side_data: raw_side_data(raw.coded_side_data, raw.nb_coded_side_data),
        }
    }
}

// SAFETY: caller passes an AVChannelLayout borrowed from a live AVCodecParameters.
unsafe fn channel_layout(raw: &ffmpeg::ffi::AVChannelLayout) -> ChannelLayout {
    use ffmpeg::ffi::AVChannelOrder;
    let mask = match raw.order {
        AVChannelOrder::AV_CHANNEL_ORDER_NATIVE | AVChannelOrder::AV_CHANNEL_ORDER_AMBISONIC => {
            // SAFETY: union arm `mask` is active for native and ambisonic channel orders.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI union read of AVChannelLayout.u.mask, which has no ffmpeg-next 9.1.0 wrapper; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next wraps AVChannelLayout
            unsafe { raw.u.mask }
        }
        _ => 0,
    };
    let custom_ids = if raw.order == AVChannelOrder::AV_CHANNEL_ORDER_CUSTOM
        // SAFETY: union arm `map` is active for custom channel order; checked for non-null and positive count.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI union pointer check on AVChannelLayout.u.map, which has no ffmpeg-next 9.1.0 wrapper; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next wraps AVChannelLayout
        && unsafe { !raw.u.map.is_null() }
        && raw.nb_channels > 0
    {
        // SAFETY: FFmpeg allocates `nb_channels` entries at `u.map` for custom channel layouts.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI slice read of AVChannelLayout.u.map entries, which has no ffmpeg-next 9.1.0 wrapper; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next wraps AVChannelLayout
        unsafe { std::slice::from_raw_parts(raw.u.map, raw.nb_channels as usize) }
            .iter()
            .map(|entry| entry.id as i32)
            .collect()
    } else {
        Vec::new()
    };
    ChannelLayout {
        order: raw.order as i32,
        nb_channels: raw.nb_channels,
        mask,
        custom_ids,
    }
}

// SAFETY: caller guarantees `ptr` is either null or points to `count` initialized AVPacketSideData items.
unsafe fn raw_side_data(
    ptr: *const ffmpeg::ffi::AVPacketSideData,
    count: std::ffi::c_int,
) -> Vec<SideDataRecord> {
    if ptr.is_null() || count <= 0 {
        return Vec::new();
    }
    // SAFETY: non-null `ptr` and positive `count` reference a contiguous FFmpeg side-data array.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI slice read of AVPacketSideData array, because ffmpeg-next 9.1.0 SideData::data() skips null checks and SideData::kind() can hit unimplemented!(); proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next soundly wraps side-data arrays
    let entries = unsafe { std::slice::from_raw_parts(ptr, count as usize) };
    let mut out = Vec::new();
    for entry in entries {
        let Some(kind) = relevant_side_data_kind(entry.type_) else {
            continue;
        };
        let bytes: Box<[u8]> = if entry.data.is_null() || entry.size == 0 {
            Box::default()
        } else {
            // SAFETY: non-null `entry.data` holds `entry.size` initialized bytes owned by FFmpeg.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI slice read of AVPacketSideData payload after null check, because ffmpeg-next 9.1.0 SideData::data() calls from_raw_parts unchecked; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next checks null side-data pointers
            unsafe { std::slice::from_raw_parts(entry.data, entry.size) }.into()
        };
        out.push(SideDataRecord { kind, data: bytes });
    }
    out.sort();
    out
}

pub(crate) fn relevant_side_data_kind(
    raw: ffmpeg::ffi::AVPacketSideDataType,
) -> Option<SideDataKind> {
    use ffmpeg::ffi::AVPacketSideDataType as Kind;
    match raw {
        Kind::AV_PKT_DATA_DISPLAYMATRIX => Some(SideDataKind::DisplayMatrix),
        Kind::AV_PKT_DATA_MASTERING_DISPLAY_METADATA => {
            Some(SideDataKind::MasteringDisplayMetadata)
        }
        Kind::AV_PKT_DATA_CONTENT_LIGHT_LEVEL => Some(SideDataKind::ContentLightLevel),
        Kind::AV_PKT_DATA_DOVI_CONF => Some(SideDataKind::DoviConf),
        Kind::AV_PKT_DATA_SKIP_SAMPLES => Some(SideDataKind::SkipSamples),
        _ => None,
    }
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
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI copy of AVStream.attached_pic into target codecpar extradata, which has no ffmpeg-next 9.1.0 wrapper; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next exposes attached_pic and owned extradata assignment
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

#[cfg(test)]
mod tests {
    use super::*;
    use ffmpeg::ffi::AVPacketSideDataType as Kind;

    #[test]
    fn skip_samples_side_data_kind_is_mapped() {
        assert_eq!(
            relevant_side_data_kind(Kind::AV_PKT_DATA_SKIP_SAMPLES),
            Some(SideDataKind::SkipSamples)
        );
        assert_eq!(relevant_side_data_kind(Kind::AV_PKT_DATA_PALETTE), None);
    }
}
