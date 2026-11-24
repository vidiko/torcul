// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use crate::{
    Error, Result,
    mp4_boxes::{self as boxes, Raw},
};
use mp4_atom::{Atom, Co64, FourCC, Stco, Tkhd};
use std::collections::BTreeMap;

#[cfg(test)]
mod mp4_movie_tests;

/// Fixed container depth: moov -> trak -> mdia -> minf -> stbl. No recursion.
pub(crate) fn movie(
    source: &Raw,
    output: &Raw,
    ids: &BTreeMap<u32, u32>,
    delta: i64,
) -> Result<Raw> {
    let source_children = boxes::children(&source.body)?;
    let mut output_children = boxes::children(&output.body)?;
    let source_tracks: BTreeMap<_, _> = source_children
        .iter()
        .filter(|b| b.kind == FourCC::new(b"trak"))
        .map(|b| Ok((track_id(b)?, b)))
        .collect::<Result<_>>()?;
    for child in output_children
        .iter_mut()
        .filter(|b| b.kind == FourCC::new(b"trak"))
    {
        let source_track = ids
            .get(&track_id(child)?)
            .and_then(|id| source_tracks.get(id))
            .copied();
        *child = track(child, source_track, delta)?;
    }
    boxes::metadata(&source_children, &mut output_children);
    boxes::pack(output.kind, &output_children)
}

fn track_id(raw: &Raw) -> Result<u32> {
    let children = boxes::children(&raw.body)?;
    let tkhd = children
        .iter()
        .find(|b| b.kind == FourCC::new(b"tkhd"))
        .ok_or_else(|| Error::Invalid("MP4 track missing tkhd".into()))?;
    Ok(Tkhd::decode_body(&mut tkhd.body.as_slice())?.track_id)
}

fn find_child<'a>(children: &'a [Raw], kind: &[u8; 4]) -> Option<&'a Raw> {
    children.iter().find(|b| b.kind == FourCC::new(kind))
}

fn track(raw: &Raw, source: Option<&Raw>, delta: i64) -> Result<Raw> {
    let mut children = boxes::children(&raw.body)?;
    let src_children = source.map(|s| boxes::children(&s.body)).transpose()?;
    let src_mdia = src_children.as_deref().and_then(|c| find_child(c, b"mdia"));
    for child in children
        .iter_mut()
        .filter(|b| b.kind == FourCC::new(b"mdia"))
    {
        *child = media(child, src_mdia, delta)?;
    }
    if let Some(src_children) = &src_children {
        boxes::metadata(src_children, &mut children);
    }
    boxes::pack(raw.kind, &children)
}

fn media(raw: &Raw, source: Option<&Raw>, delta: i64) -> Result<Raw> {
    let mut children = boxes::children(&raw.body)?;
    let src_children = source.map(|s| boxes::children(&s.body)).transpose()?;
    let src_minf = src_children.as_deref().and_then(|c| find_child(c, b"minf"));
    for child in children
        .iter_mut()
        .filter(|b| b.kind == FourCC::new(b"minf"))
    {
        *child = information(child, src_minf, delta)?;
    }
    boxes::pack(raw.kind, &children)
}

fn information(raw: &Raw, source: Option<&Raw>, delta: i64) -> Result<Raw> {
    let mut children = boxes::children(&raw.body)?;
    let src_children = source.map(|s| boxes::children(&s.body)).transpose()?;
    let src_stbl = src_children.as_deref().and_then(|c| find_child(c, b"stbl"));
    for child in children
        .iter_mut()
        .filter(|b| b.kind == FourCC::new(b"stbl"))
    {
        *child = samples(child, src_stbl, delta)?;
    }
    boxes::pack(raw.kind, &children)
}

fn samples(raw: &Raw, source: Option<&Raw>, delta: i64) -> Result<Raw> {
    let mut children = boxes::children(&raw.body)?;
    let src_children = source.map(|s| boxes::children(&s.body)).transpose()?;
    let src_stsd = src_children.as_deref().and_then(|c| find_child(c, b"stsd"));
    for child in &mut children {
        if child.kind == FourCC::new(b"stsd") {
            if let Some(original_stsd) = src_stsd {
                *child = original_stsd.clone();
            }
            continue;
        }
        let entries = if child.kind == FourCC::new(b"stco") {
            Stco::decode_body(&mut child.body.as_slice())?
                .entries
                .into_iter()
                .map(u64::from)
                .collect()
        } else if child.kind == FourCC::new(b"co64") {
            Co64::decode_body(&mut child.body.as_slice())?.entries
        } else {
            continue;
        };
        let entries = entries
            .into_iter()
            .map(|offset| {
                offset
                    .checked_add_signed(delta)
                    .ok_or_else(|| Error::Invalid("MP4 chunk offset overflow".into()))
            })
            .collect::<Result<_>>()?;
        let mut body = Vec::new();
        Co64 { entries }.encode_body(&mut body)?;
        *child = Raw {
            kind: FourCC::new(b"co64"),
            body,
        };
    }
    boxes::pack(raw.kind, &children)
}
