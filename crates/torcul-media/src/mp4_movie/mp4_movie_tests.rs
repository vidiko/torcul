// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use super::*;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn raw_tkhd(track_id: u32) -> TestResult<Raw> {
    let mut body = Vec::new();
    Tkhd {
        track_id,
        ..Tkhd::default()
    }
    .encode_body(&mut body)?;
    Ok(Raw {
        kind: FourCC::new(b"tkhd"),
        body,
    })
}

fn raw_stco(entries: &[u32]) -> TestResult<Raw> {
    let mut body = Vec::new();
    Stco {
        entries: entries.to_vec(),
    }
    .encode_body(&mut body)?;
    Ok(Raw {
        kind: FourCC::new(b"stco"),
        body,
    })
}

fn raw_co64(entries: &[u64]) -> TestResult<Raw> {
    let mut body = Vec::new();
    Co64 {
        entries: entries.to_vec(),
    }
    .encode_body(&mut body)?;
    Ok(Raw {
        kind: FourCC::new(b"co64"),
        body,
    })
}

fn build_moov(track_id: u32, chunk_table: Raw) -> TestResult<Raw> {
    let stsd = Raw {
        kind: FourCC::new(b"stsd"),
        body: vec![0, 0, 0, 0, 0, 0, 0, 1],
    };
    let stbl = boxes::pack(FourCC::new(b"stbl"), &[stsd, chunk_table])?;
    let minf = boxes::pack(FourCC::new(b"minf"), &[stbl])?;
    let mdia = boxes::pack(FourCC::new(b"mdia"), &[minf])?;
    let trak = boxes::pack(FourCC::new(b"trak"), &[raw_tkhd(track_id)?, mdia])?;
    Ok(boxes::pack(FourCC::new(b"moov"), &[trak])?)
}

fn child(parent: &Raw, kind: &[u8; 4]) -> TestResult<Raw> {
    let children = boxes::children(&parent.body)?;
    find_child(&children, kind)
        .cloned()
        .ok_or_else(|| format!("missing {} box", String::from_utf8_lossy(kind)).into())
}

fn extract_co64_entries(moov: &Raw) -> TestResult<Vec<u64>> {
    let mut stbl = moov.clone();
    for kind in [b"trak", b"mdia", b"minf", b"stbl"] {
        stbl = child(&stbl, kind)?;
    }
    if child(&stbl, b"stco").is_ok() {
        return Err("rewritten stbl must not keep stco".into());
    }
    let co64 = child(&stbl, b"co64")?;
    Ok(Co64::decode_body(&mut co64.body.as_slice())?.entries)
}

#[test]
fn movie_rewrites_stco_and_co64_for_positive_negative_and_zero_deltas() -> TestResult<()> {
    let ids = BTreeMap::from([(1, 1)]);
    let src = build_moov(1, raw_stco(&[100, 200])?)?;

    let out_stco = build_moov(1, raw_stco(&[100, 250])?)?;
    let pos_stco = movie(&src, &out_stco, &ids, 24)?;
    assert_eq!(extract_co64_entries(&pos_stco)?, vec![124, 274]);
    let neg_stco = movie(&src, &out_stco, &ids, -16)?;
    assert_eq!(extract_co64_entries(&neg_stco)?, vec![84, 234]);
    let zero_stco = movie(&src, &out_stco, &ids, 0)?;
    assert_eq!(extract_co64_entries(&zero_stco)?, vec![100, 250]);

    let out_co64 = build_moov(1, raw_co64(&[5_000_000_000, 6_000_000_000])?)?;
    let pos_co64 = movie(&src, &out_co64, &ids, 32)?;
    assert_eq!(
        extract_co64_entries(&pos_co64)?,
        vec![5_000_000_032, 6_000_000_032]
    );
    let neg_co64 = movie(&src, &out_co64, &ids, -32)?;
    assert_eq!(
        extract_co64_entries(&neg_co64)?,
        vec![4_999_999_968, 5_999_999_968]
    );
    let zero_co64 = movie(&src, &out_co64, &ids, 0)?;
    assert_eq!(
        extract_co64_entries(&zero_co64)?,
        vec![5_000_000_000, 6_000_000_000]
    );
    Ok(())
}

#[test]
fn movie_chunk_offset_boundaries_promote_u32_max_and_reject_overflow() -> TestResult<()> {
    let ids = BTreeMap::from([(1, 1)]);
    let src = build_moov(1, raw_stco(&[10])?)?;

    let at_u32_max = build_moov(1, raw_stco(&[u32::MAX])?)?;
    let promoted = movie(&src, &at_u32_max, &ids, 16)?;
    assert_eq!(
        extract_co64_entries(&promoted)?,
        vec![u64::from(u32::MAX) + 16]
    );

    let exact_zero = build_moov(1, raw_stco(&[12])?)?;
    assert_eq!(
        extract_co64_entries(&movie(&src, &exact_zero, &ids, -12)?)?,
        vec![0]
    );

    let err = movie(&src, &exact_zero, &ids, -13).unwrap_err();
    assert!(err.to_string().contains("MP4 chunk offset overflow"));

    let at_u64_max = build_moov(1, raw_co64(&[u64::MAX])?)?;
    let err = movie(&src, &at_u64_max, &ids, 1).unwrap_err();
    assert!(err.to_string().contains("MP4 chunk offset overflow"));
    Ok(())
}
