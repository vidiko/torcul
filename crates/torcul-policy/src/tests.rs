// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use super::*;

fn track(index: usize, kind: TrackKind, language: Option<&str>) -> TrackInfo {
    TrackInfo {
        index,
        kind,
        codec: "fixture".into(),
        language: language.map(str::to_owned),
        title: None,
        dispositions: BTreeSet::new(),
        attached_pic: false,
    }
}
fn audio(langs: &[Option<&str>], original: Option<&str>, anime: bool) -> Plan {
    let facts = FileFacts {
        tracks: langs
            .iter()
            .enumerate()
            .map(|(i, l)| track(i, TrackKind::Audio, *l))
            .collect(),
        container: "matroska".into(),
    };
    plan(
        &facts,
        &TitleMeta {
            original: original.map(str::to_owned),
            anime,
        },
    )
}
#[test]
fn language_aliases_and_bcp47() {
    for s in ["en", "eng", "ENG", "en-US", "En-Latn-US"] {
        assert_eq!(normalize_language(s).as_deref(), Some("eng"));
    }
    for (b, t) in [
        ("ger", "deu"),
        ("fre", "fra"),
        ("chi", "zho"),
        ("cze", "ces"),
        ("wel", "cym"),
        ("slo", "slk"),
        ("tib", "bod"),
        ("gre", "ell"),
        ("dut", "nld"),
        ("ice", "isl"),
        ("alb", "sqi"),
        ("arm", "hye"),
        ("baq", "eus"),
        ("bur", "mya"),
        ("geo", "kat"),
        ("mac", "mkd"),
        ("mao", "mri"),
        ("may", "msa"),
        ("per", "fas"),
        ("rum", "ron"),
    ] {
        assert_eq!(normalize_language(b).as_deref(), Some(t));
    }
    assert_eq!(normalize_language("nonsense"), None);
}
#[test]
fn preferred_original_and_anime() {
    assert_eq!(
        audio(
            &[Some("fr"), Some("en"), Some("de"), Some("ja")],
            Some("jpn"),
            false
        )
        .kept_indices(),
        [1, 2]
    );
    assert_eq!(
        audio(
            &[Some("fr"), Some("en"), Some("de"), Some("ja")],
            Some("jpn"),
            true
        )
        .kept_indices(),
        [1, 2, 3]
    );
    assert_eq!(
        audio(&[Some("fr"), Some("it")], Some("fre"), false).kept_indices(),
        [0]
    );
    assert_eq!(
        audio(&[Some("en"), Some("de")], Some("en"), true).kept_indices(),
        [0, 1]
    );
}
#[test]
fn fallback_and_unknown_original() {
    let p = audio(&[Some("fr"), Some("it")], None, false);
    assert_eq!(p.kept_indices(), [0, 1]);
    assert_eq!(p.warnings, ["original-language-unknown"]);
    assert!(!p.needs_remux);
    let p = audio(&[Some("fr"), Some("it")], Some("ko"), false);
    assert_eq!(p.warnings, ["no-audio-matched"]);
    assert!(!p.needs_remux);
}
#[test]
fn undetermined_tracks_and_empty_tags() {
    let p = audio(
        &[None, Some(""), Some("und"), Some("UND"), Some("spa")],
        Some("en"),
        false,
    );
    assert_eq!(p.kept_indices(), [0, 1, 2, 3]);
    assert_eq!(p.warnings, ["language-undetermined"]);
}
#[test]
fn subtitles_variants_commentary_and_default_move() {
    let mut tracks = vec![
        track(0, TrackKind::Audio, Some("fr")),
        track(1, TrackKind::Audio, Some("ENG")),
    ];
    tracks[0].dispositions.insert("default".into());
    tracks[1].dispositions.insert("comment".into());
    for (i, l) in ["en", "de", "ja", "und"].iter().enumerate() {
        let mut t = track(i + 2, TrackKind::Subtitle, Some(l));
        t.dispositions
            .extend(["forced".into(), "hearing_impaired".into(), "comment".into()]);
        tracks.push(t);
    }
    let p = plan(
        &FileFacts {
            tracks,
            container: "mkv".into(),
        },
        &TitleMeta::default(),
    );
    assert_eq!(p.kept_indices(), [1, 2, 3, 5]);
    assert_eq!(p.default_audio_change, Some((0, 1)));
    assert_eq!(p.warnings, ["default-audio-moved", "language-undetermined"]);
}
#[test]
fn structural_streams_no_audio_or_subtitles() {
    for kind in [
        TrackKind::Video,
        TrackKind::Attachment,
        TrackKind::Data,
        TrackKind::Other,
    ] {
        let mut t = track(0, kind, None);
        t.attached_pic = kind == TrackKind::Video;
        let p = plan(
            &FileFacts {
                tracks: vec![t],
                container: "mkv".into(),
            },
            &TitleMeta::default(),
        );
        assert_eq!(p.kept_indices(), [0]);
        assert!(p.warnings.is_empty());
        assert!(!p.needs_remux);
    }
    let p = plan(
        &FileFacts {
            tracks: vec![],
            container: "mkv".into(),
        },
        &TitleMeta::default(),
    );
    assert!(!p.needs_remux);
}

#[test]
fn all_undetermined_and_invalid_original() {
    let tracks = vec![
        track(0, TrackKind::Audio, Some("und")),
        track(1, TrackKind::Subtitle, Some("")),
    ];
    let p = plan(
        &FileFacts {
            tracks,
            container: "mkv".into(),
        },
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    );
    assert_eq!(p.kept_indices(), [0, 1]);
    assert_eq!(p.warnings, ["language-undetermined"]);
    assert!(!p.needs_remux);
    let p = audio(&[Some("zzzz")], Some("invalid"), false);
    assert_eq!(p.kept_indices(), [0]);
    assert_eq!(p.warnings, ["no-audio-matched"]);
}
