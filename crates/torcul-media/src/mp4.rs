// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{
    Error, Result,
    error::fs,
    inspect,
    io::{Deadline, TimedFile, open_regular},
    mp4_boxes as boxes, mp4_movie,
};
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use torcul_policy::Plan;

/// Preserve opaque metadata using mp4-atom's pinned box schema. FFmpeg's mdta
/// option discards covers, so keep default muxing and transplant metadata only.
/// Output stays unfragmented: ftyp first, moov last. Offset changes use Co64.
pub(crate) fn preserve(src: &Path, path: &Path, plan: &Plan, deadline: Deadline) -> Result<()> {
    let ids = track_ids(src, path, plan, deadline)?;
    let mut source = TimedFile {
        file: fs("open source MP4 metadata", open_regular(src, deadline))?,
        deadline,
    };
    let mut output = TimedFile {
        file: fs("open staged MP4 metadata", open_regular(path, deadline))?,
        deadline,
    };
    let source_index = boxes::index(&mut source)?;
    let output_index = boxes::index(&mut output)?;
    let old_ftyp = boxes::get(&output_index, b"ftyp")?;
    let old_moov = boxes::get(&output_index, b"moov")?;
    if old_ftyp.start != 0
        || output_index
            .last()
            .is_none_or(|b| b.start != old_moov.start)
    {
        return Err(Error::Invalid(
            "expected staged MP4 ftyp first and moov last".into(),
        ));
    }
    let new_ftyp = boxes::encode(&boxes::load(
        &mut source,
        boxes::get(&source_index, b"ftyp")?,
    )?)?;
    let delta = new_ftyp.len() as i64 - old_ftyp.end as i64;
    let source_moov = boxes::load(&mut source, boxes::get(&source_index, b"moov")?)?;
    let output_moov = boxes::load(&mut output, old_moov)?;
    let new_moov = boxes::encode(&mp4_movie::movie(&source_moov, &output_moov, &ids, delta)?)?;
    replace(
        path,
        &mut output,
        old_ftyp.end,
        old_moov.start,
        &new_ftyp,
        &new_moov,
    )
}
fn track_ids(
    src: &Path,
    dst: &Path,
    plan: &Plan,
    deadline: Deadline,
) -> Result<BTreeMap<u32, u32>> {
    let source = inspect::open(src, deadline)?;
    let output = inspect::open(dst, deadline)?;
    let kept = plan.kept_indices();
    if output.nb_streams() as usize != kept.len() {
        return Err(Error::Invalid(
            "staged MP4 lost streams before metadata preservation".into(),
        ));
    }
    output
        .streams()
        .zip(kept)
        .filter(|(s, _)| {
            !s.disposition()
                .contains(ffmpeg_next::format::stream::Disposition::ATTACHED_PIC)
        })
        .map(|(s, from)| {
            let original = source
                .stream(from)
                .ok_or_else(|| Error::Invalid("MP4 plan index invalid".into()))?;
            let to = u32::try_from(s.id())
                .map_err(|_| Error::Invalid("negative MP4 track id".into()))?;
            let from = u32::try_from(original.id())
                .map_err(|_| Error::Invalid("negative source track id".into()))?;
            Ok((to, from))
        })
        .collect()
}
fn replace(
    path: &Path,
    input: &mut TimedFile,
    begin: u64,
    end: u64,
    ftyp: &[u8],
    moov: &[u8],
) -> Result<()> {
    let deadline = input.deadline;
    let parent = path.parent().unwrap_or(Path::new("."));
    let temp = fs(
        "create MP4 metadata replacement",
        deadline.run(|| tempfile::NamedTempFile::new_in(parent)),
    )?;
    let mut writer = TimedFile {
        file: fs(
            "clone MP4 replacement handle",
            deadline.run(|| temp.as_file().try_clone()),
        )?,
        deadline,
    };
    fs("write source MP4 ftyp", writer.write_all(ftyp))?;
    copy_range(input, &mut writer, begin, end)?;
    fs("write MP4 movie metadata", writer.write_all(moov))?;
    fs(
        "sync MP4 replacement",
        deadline.run(|| writer.file.sync_all()),
    )?;
    drop(writer);
    deadline.check().map_err(|source| Error::Io {
        operation: "MP4 persist deadline",
        source,
    })?;
    temp.persist(path).map_err(|e| Error::Io {
        operation: "persist MP4 metadata",
        source: e.error,
    })?;
    fs("MP4 persist deadline", deadline.check())?;
    Ok(())
}
fn copy_range(input: &mut TimedFile, output: &mut TimedFile, begin: u64, end: u64) -> Result<()> {
    let size = end
        .checked_sub(begin)
        .ok_or_else(|| Error::Invalid("invalid MP4 copy range".into()))?;
    fs("seek MP4 media data", input.seek(SeekFrom::Start(begin)))?;
    let mut remaining = size;
    let mut buffer = vec![0; 1024 * 1024];
    for _ in 0..=size / (buffer.len() as u64) {
        if remaining == 0 {
            return Ok(());
        }
        let length = remaining.min(buffer.len() as u64) as usize;
        fs(
            "read MP4 media data",
            input.read_exact(&mut buffer[..length]),
        )?;
        fs("copy MP4 media data", output.write_all(&buffer[..length]))?;
        remaining -= length as u64;
    }
    if remaining != 0 {
        return Err(Error::Invalid("MP4 copy byte bound exceeded".into()));
    }
    Ok(())
}
