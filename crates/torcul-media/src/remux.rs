// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{
    Error, Result,
    error::{av, fs},
    inspect,
    io::{Deadline, TimedFile},
    packets,
};
use ffmpeg_next::{self as ffmpeg, format};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};
use torcul_policy::Plan;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemuxStats {
    pub packets_written: u64,
    pub bytes_written: u64,
}

struct Output {
    inner: format::context::Output,
    // Stable address; dropped after inner so callback remains valid on close.
    _deadline: Box<Deadline>,
}
// SAFETY: only installed with a live boxed Deadline owned by Output.
unsafe extern "C" fn interrupt(opaque: *mut std::ffi::c_void) -> i32 {
    // SAFETY: opaque installed from Box<Deadline>; Output drops context first.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI callback borrow of AVIOInterruptCB opaque pointer, because ffmpeg-next 9.1.0 provides no output interrupt callback wrapper; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next wraps output interrupt callbacks
    i32::from(unsafe { &*opaque.cast::<Deadline>() }.expired())
}

fn output(file: File, container: &str, deadline: Deadline) -> Result<Output> {
    let io = av(
        "create seekable output I/O",
        format::context::StreamIo::from_write_seek(TimedFile { file, deadline }),
    )?;
    let mut inner = av(
        "allocate output",
        format::output_to_stream(io, None, Some(container)),
    )?;
    let mut guard = Box::new(deadline);
    // SAFETY: context exclusively owned; stable boxed deadline outlives context;
    // custom IO has its own deadline checks on every write/flush/seek callback.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI assignment of AVFormatContext.interrupt_callback, which has no ffmpeg-next 9.1.0 wrapper on Output; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next wraps output interrupt_callback setup
    unsafe {
        (*inner.as_mut_ptr()).interrupt_callback = ffmpeg::ffi::AVIOInterruptCB {
            callback: Some(interrupt),
            opaque: (&raw mut *guard).cast(),
        };
    }
    Ok(Output {
        inner,
        _deadline: guard,
    })
}

/// Stream copy into a new staging file. Errors remove partial output.
pub fn remux(src: &Path, plan: &Plan, staging_path: &Path) -> Result<RemuxStats> {
    let deadline = Deadline::new();
    let mut input = inspect::open(src, deadline)?;
    validate_plan(&input, plan)?;
    if !plan.needs_remux {
        return Ok(RemuxStats {
            packets_written: 0,
            bytes_written: 0,
        });
    }
    let container = container(&input)?;
    fs("staging creation deadline", deadline.check())?;
    let file = fs(
        "create staging (must not exist)",
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging_path),
    )?;
    // Retain ownership before checking the post-open deadline, so even a slow
    // successful create is removed if the operation has already timed out.
    let result = fs("staging creation deadline", deadline.check())
        .and_then(|()| run(&mut input, plan, file, container, deadline))
        .and_then(|stats| {
            if container == "mp4" {
                crate::mp4::preserve(src, staging_path, plan, deadline)?;
            }
            Ok(stats)
        });
    if result.is_err() {
        match deadline.remove_partial(staging_path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(source) => {
                return Err(Error::Io {
                    operation: "remove partial staging",
                    source,
                });
            }
        }
    }
    result
}

pub(crate) fn validate_plan(input: &format::context::Input, plan: &Plan) -> Result<()> {
    if plan.decisions.len() != input.nb_streams() as usize
        || plan.decisions.iter().enumerate().any(|(i, d)| i != d.index)
        || plan.kept_indices().is_empty()
    {
        return Err(Error::Invalid(
            "plan must cover every source stream once, in order".into(),
        ));
    }
    if plan.needs_remux
        != (plan.decisions.iter().any(|d| !d.keep) || plan.default_audio_change.is_some())
    {
        return Err(Error::Invalid("inconsistent needs_remux".into()));
    }
    if let Some((_, to)) = plan.default_audio_change {
        let Some(stream) = input.stream(to) else {
            return Err(Error::Invalid(
                "default_audio_change target stream out of range".into(),
            ));
        };
        if !plan.decisions[to].keep || stream.parameters().medium() != ffmpeg::media::Type::Audio {
            return Err(Error::Invalid(
                "default_audio_change target must be a kept audio stream".into(),
            ));
        }
    }
    if matches!(input.format().name(), "mov,mp4,m4a,3gp,3g2,mj2" | "mp4")
        && input.streams().any(|s| {
            !plan.decisions[s.index()].keep
                && s.disposition()
                    .contains(format::stream::Disposition::ATTACHED_PIC)
        })
    {
        return Err(Error::Invalid(
            "dropping an MP4 cover is unsupported: restored metadata would keep covr".into(),
        ));
    }
    Ok(())
}
fn container(input: &format::context::Input) -> Result<&'static str> {
    match input.format().name() {
        "matroska,webm" | "matroska" => Ok("matroska"),
        "mov,mp4,m4a,3gp,3g2,mj2" | "mp4" => Ok("mp4"),
        other => Err(Error::Invalid(format!("unsupported container: {other}"))),
    }
}

fn run(
    input: &mut format::context::Input,
    plan: &Plan,
    file: File,
    container: &str,
    deadline: Deadline,
) -> Result<RemuxStats> {
    let sync = fs("clone staging handle", deadline.run(|| file.try_clone()))?;
    let mut out = output(file, container, deadline)?;
    let mapping = streams(input, &mut out.inner, plan)?;
    copy_chapters(input, &mut out.inner)?;
    write_header(input, &mut out.inner)?;
    let stats = copy_packets(input, &mut out.inner, &mapping, deadline)?;
    av("write output trailer", out.inner.write_trailer())?;
    // SAFETY: live custom-IO output owns non-null pb. Trailer flushed buffer;
    // inspect AVIO's latched write error before destructor can discard it.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI read of AVIOContext.error on custom output I/O, which has no ffmpeg-next 9.1.0 wrapper; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next exposes AVIOContext error state
    let code = unsafe { (*(*out.inner.as_ptr()).pb).error };
    if code < 0 {
        return av("flush output", Err(ffmpeg::Error::from(code)));
    }
    drop(out);
    fs("fsync staging", deadline.run(|| sync.sync_all()))?;
    Ok(stats)
}

fn streams(
    input: &format::context::Input,
    output: &mut format::context::Output,
    plan: &Plan,
) -> Result<Vec<Option<usize>>> {
    let mut mapping = vec![None; input.nb_streams() as usize];
    let is_matroska = output.format().name() == "matroska";
    for source in input.streams().filter(|s| plan.decisions[s.index()].keep) {
        let mut target = av(
            "add output stream",
            output.add_stream(ffmpeg::encoder::find(ffmpeg::codec::Id::None)),
        )?;
        mapping[source.index()] = Some(target.index());
        copy_parameters(&source, &mut target, plan)?;
        let mut metadata = source.metadata().to_owned();
        // MP4 demuxes track titles as "name"; its muxer consumes "title".
        if !is_matroska
            && metadata.get("title").is_none()
            && let Some(name) = source.metadata().get("name")
        {
            metadata.set("title", name);
        }
        target.set_metadata(metadata);
        target.set_time_base(source.time_base());
        target.set_avg_frame_rate(source.avg_frame_rate());
        target.set_rate(source.rate());
        if is_matroska
            && source
                .disposition()
                .contains(format::stream::Disposition::ATTACHED_PIC)
        {
            packets::picture_attachment(&source, &mut target)?;
            mapping[source.index()] = None;
        }
    }
    Ok(mapping)
}
fn copy_parameters(
    source: &ffmpeg::Stream<'_>,
    target: &mut ffmpeg::StreamMut<'_>,
    plan: &Plan,
) -> Result<()> {
    // SAFETY: source parameters live throughout call; target owns fresh codecpar.
    // Deep copy includes extradata and coded side data; errors checked.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI call to avcodec_parameters_copy and AVStream disposition/codec_tag setup, because ffmpeg-next 9.1.0 discards the copy result code and lacks disposition setters; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next checks avcodec_parameters_copy errors and exposes stream disposition setters
    let result = unsafe {
        let dst = target.as_mut_ptr();
        let src = source.as_ptr();
        let result = ffmpeg::ffi::avcodec_parameters_copy((*dst).codecpar, (*src).codecpar);
        (*(*dst).codecpar).codec_tag = 0;
        (*dst).disposition = (*src).disposition;
        if plan
            .default_audio_change
            .is_some_and(|(_, to)| to == source.index())
        {
            (*dst).disposition |= ffmpeg::ffi::AV_DISPOSITION_DEFAULT;
        }
        result
    };
    av(
        "copy codec parameters",
        if result < 0 {
            Err(ffmpeg::Error::from(result))
        } else {
            Ok(())
        },
    )
}
fn copy_chapters(
    input: &format::context::Input,
    output: &mut format::context::Output,
) -> Result<()> {
    for source in input.chapters() {
        let mut target = av(
            "copy chapter",
            output.add_chapter(
                source.id(),
                source.time_base(),
                source.start(),
                source.end(),
                source.metadata().get("title").unwrap_or(""),
            ),
        )?;
        for (key, value) in source.metadata().iter() {
            target.set_metadata(key, value);
        }
    }
    Ok(())
}
fn copy_packets(
    input: &mut format::context::Input,
    output: &mut format::context::Output,
    mapping: &[Option<usize>],
    deadline: Deadline,
) -> Result<RemuxStats> {
    let mut stats = RemuxStats {
        packets_written: 0,
        bytes_written: 0,
    };
    for _ in 0..packets::MAX_PACKETS {
        let Some(mut packet) = packets::read(input, deadline)? else {
            return Ok(stats);
        };
        let index = packet.stream();
        let Some(to) = mapping.get(index).copied().flatten() else {
            continue;
        };
        let source = input
            .stream(index)
            .ok_or_else(|| Error::Invalid("source stream missing".into()))?;
        let target = output
            .stream(to)
            .ok_or_else(|| Error::Invalid("output stream missing".into()))?;
        stats.packets_written += 1;
        stats.bytes_written += packet.size() as u64;
        packet.rescale_ts(source.time_base(), target.time_base());
        packet.set_position(-1);
        packet.set_stream(to);
        av("write interleaved packet", packet.write_interleaved(output))?;
    }
    Err(Error::Invalid(format!(
        "packet bound {} exceeded",
        packets::MAX_PACKETS
    )))
}

fn write_header(
    input: &format::context::Input,
    output: &mut format::context::Output,
) -> Result<()> {
    output.set_metadata(input.metadata().to_owned());
    // SAFETY: output exclusively owned, custom AVIO live, no options. Explicit
    // init runs the muxer's metadata rewrite before restoring source tags.
    let result =
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI call to avformat_init_output, which has no ffmpeg-next 9.1.0 wrapper; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next wraps avformat_init_output
        unsafe { ffmpeg::ffi::avformat_init_output(output.as_mut_ptr(), std::ptr::null_mut()) };
    if result < 0 {
        return av("initialize muxer", Err(ffmpeg::Error::from(result)));
    }
    // SAFETY: free only output's owned dictionary, then deep-copy input's live
    // dictionary. Context owns replacement; allocation error propagated.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage -- FFI call to av_dict_free and av_dict_copy on AVFormatContext.metadata, because ffmpeg-next 9.1.0 Output::set_metadata leaks an existing dictionary and ignores copy errors; proof in the SAFETY comment above (HISS-09); category D, expires 2027-03-24, ends when ffmpeg-next replaces output metadata without leaking and propagates av_dict_copy errors
    let result = unsafe {
        let dst = output.as_mut_ptr();
        ffmpeg::ffi::av_dict_free(&raw mut (*dst).metadata);
        ffmpeg::ffi::av_dict_copy(&raw mut (*dst).metadata, input.metadata().as_ptr(), 0)
    };
    if result < 0 {
        return av("restore global metadata", Err(ffmpeg::Error::from(result)));
    }
    av("write output header", output.write_header())?;
    Ok(())
}
