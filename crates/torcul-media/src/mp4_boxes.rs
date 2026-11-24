// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{Error, Result, error::fs, io::TimedFile};
use mp4_atom::{FourCC, Header, ReadFrom, WriteTo};
use std::io::{Cursor, Read, Seek, SeekFrom};

pub(crate) const MAX_BOX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BOXES: usize = 1_000_000;

#[derive(Clone, Debug)]
pub(crate) struct Raw {
    pub kind: FourCC,
    pub body: Vec<u8>,
}
#[derive(Debug)]
pub(crate) struct Location {
    pub kind: FourCC,
    pub start: u64,
    pub body: u64,
    pub end: u64,
}

pub(crate) fn index(file: &mut TimedFile) -> Result<Vec<Location>> {
    let size = fs("stat MP4", file.deadline.run(|| file.file.metadata()))?.len();
    index_with_size(file, size)
}

pub(crate) fn index_with_size(file: &mut TimedFile, size: u64) -> Result<Vec<Location>> {
    let mut position = fs("seek MP4 start", file.seek(SeekFrom::Start(0)))?;
    let mut boxes = Vec::new();
    for _ in 0..MAX_BOXES {
        if position == size {
            return Ok(boxes);
        }
        let header = Header::read_from(file)?;
        let body = fs("MP4 header position", file.stream_position())?;
        let remaining = size
            .checked_sub(body)
            .ok_or_else(|| Error::Invalid("MP4 box header exceeds file".into()))?;
        let length = header.size.map_or(remaining, |n| n as u64);
        let end = body
            .checked_add(length)
            .filter(|end| *end <= size)
            .ok_or_else(|| Error::Invalid("MP4 box exceeds file".into()))?;
        boxes.push(Location {
            kind: header.kind,
            start: position,
            body,
            end,
        });
        position = fs("seek next MP4 box", file.seek(SeekFrom::Start(end)))?;
    }
    Err(Error::Invalid("MP4 box count bound exceeded".into()))
}
pub(crate) fn load(file: &mut TimedFile, location: &Location) -> Result<Raw> {
    let size = location.end - location.body;
    if size > MAX_BOX_BYTES {
        return Err(Error::Invalid("MP4 metadata exceeds 64 MiB bound".into()));
    }
    fs(
        "seek MP4 metadata",
        file.seek(SeekFrom::Start(location.body)),
    )?;
    let mut body = vec![0; size as usize];
    fs("read MP4 metadata", file.read_exact(&mut body))?;
    Ok(Raw {
        kind: location.kind,
        body,
    })
}
pub(crate) fn children(body: &[u8]) -> Result<Vec<Raw>> {
    let mut cursor = Cursor::new(body);
    let mut boxes = Vec::new();
    for _ in 0..MAX_BOXES {
        if cursor.position() == body.len() as u64 {
            return Ok(boxes);
        }
        let header = Header::read_from(&mut cursor)?;
        let remaining = body
            .len()
            .checked_sub(cursor.position() as usize)
            .ok_or_else(|| Error::Invalid("MP4 child header exceeds parent".into()))?;
        let size = header.size.unwrap_or(remaining);
        if size > remaining {
            return Err(Error::Invalid("MP4 child exceeds parent".into()));
        }
        let mut data = vec![0; size];
        fs("read MP4 child", cursor.read_exact(&mut data))?;
        boxes.push(Raw {
            kind: header.kind,
            body: data,
        });
    }
    Err(Error::Invalid("MP4 child count bound exceeded".into()))
}
pub(crate) fn pack(kind: FourCC, children: &[Raw]) -> Result<Raw> {
    let mut body = Vec::new();
    for child in children {
        body.extend(encode(child)?);
    }
    if body.len() as u64 > MAX_BOX_BYTES {
        return Err(Error::Invalid(
            "rewritten MP4 metadata exceeds 64 MiB bound".into(),
        ));
    }
    Ok(Raw { kind, body })
}
pub(crate) fn encode(raw: &Raw) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    Header {
        kind: raw.kind,
        size: Some(raw.body.len()),
    }
    .write_to(&mut data)?;
    data.extend_from_slice(&raw.body);
    Ok(data)
}
pub(crate) fn get<'a>(boxes: &'a [Location], kind: &[u8; 4]) -> Result<&'a Location> {
    boxes
        .iter()
        .find(|b| b.kind == FourCC::new(kind))
        .ok_or_else(|| Error::Invalid(format!("MP4 missing {}", String::from_utf8_lossy(kind))))
}
pub(crate) fn metadata(source: &[Raw], target: &mut Vec<Raw>) {
    target.retain(|b| !is_metadata(b.kind));
    target.extend(source.iter().filter(|b| is_metadata(b.kind)).cloned());
}
fn is_metadata(kind: FourCC) -> bool {
    kind == FourCC::new(b"udta") || kind == FourCC::new(b"meta")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::Deadline;
    use std::fs::File;

    fn timed_over_bytes(dir: &std::path::Path, name: &str, bytes: &[u8]) -> TimedFile {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        TimedFile {
            file: File::open(&path).unwrap(),
            deadline: Deadline::new(),
        }
    }

    #[test]
    fn index_handles_valid_boxes_empty_and_oversized_headers() {
        let temp = tempfile::tempdir().unwrap();
        let mut valid_bytes = encode(&Raw {
            kind: FourCC::new(b"ftyp"),
            body: b"isom\0\0\x02\0".to_vec(),
        })
        .unwrap();
        valid_bytes.extend(
            encode(&Raw {
                kind: FourCC::new(b"free"),
                body: Vec::new(),
            })
            .unwrap(),
        );
        let mut valid = timed_over_bytes(temp.path(), "valid.mp4", &valid_bytes);
        let indexed = index(&mut valid).unwrap();
        assert_eq!(indexed.len(), 2);
        assert_eq!(indexed[0].kind, FourCC::new(b"ftyp"));
        assert_eq!(indexed[1].body, indexed[1].end);

        let mut empty = timed_over_bytes(temp.path(), "empty.mp4", b"");
        assert!(index(&mut empty).unwrap().is_empty());

        let mut truncated = timed_over_bytes(temp.path(), "short.mp4", b"\0\0\0\x10");
        assert!(index(&mut truncated).is_err());

        let mut open_ended = timed_over_bytes(temp.path(), "open.mp4", b"\0\0\0\0mdat");
        let err = index_with_size(&mut open_ended, 4).unwrap_err();
        assert!(err.to_string().contains("MP4 box header exceeds file"));

        let mut sized = timed_over_bytes(temp.path(), "sized.mp4", b"\0\0\0\x08free");
        let err = index_with_size(&mut sized, 4).unwrap_err();
        assert!(err.to_string().contains("MP4 box header exceeds file"));
    }
}
