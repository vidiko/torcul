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

fn try_audio(langs: &[Option<&str>], original: Option<&str>, anime: bool) -> Result<Plan> {
    let facts = FileFacts {
        tracks: langs
            .iter()
            .enumerate()
            .map(|(i, l)| track(i, TrackKind::Audio, *l))
            .collect(),
        container: "matroska".into(),
        chapters: 0,
    };
    plan(
        &facts,
        &TitleMeta {
            original: original.map(str::to_owned),
            anime,
        },
    )
}

fn audio(langs: &[Option<&str>], original: Option<&str>, anime: bool) -> Plan {
    try_audio(langs, original, anime).unwrap()
}

#[test]
fn language_aliases_and_bcp47() {
    for s in ["en", "eng", "ENG", "en-US", "En-Latn-US"] {
        assert_eq!(normalize_language(s).as_deref(), Some("eng"));
    }
    for (bib, iso1, expected) in [
        ("ger", "de", "deu"),
        ("fre", "fr", "fra"),
        ("chi", "zh", "zho"),
        ("dut", "nl", "nld"),
        ("cze", "cs", "ces"),
    ] {
        assert_eq!(normalize_language(bib).as_deref(), Some(expected));
        assert_eq!(normalize_language(bib), normalize_language(iso1));
    }
    for (b, t) in [
        ("wel", "cym"),
        ("slo", "slk"),
        ("tib", "bod"),
        ("gre", "ell"),
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
    assert_eq!(normalize_language("ber").as_deref(), Some("ber"));
    for invalid in ["", "xx", "qzz", "zzz", "english", "en-"] {
        assert_eq!(normalize_language(invalid), None, "{invalid}");
    }
}

#[test]
fn preferred_original_and_anime() {
    let movie = audio(
        &[Some("fr"), Some("en"), Some("de"), Some("ja")],
        Some("jpn"),
        false,
    );
    assert_eq!(movie.kept_indices(), [1, 2]);
    assert_eq!(movie.dropped_indices(), [0, 3]);
    for d in &movie.decisions {
        assert_eq!(d.lid.as_deref(), (!d.keep).then_some("pending"));
    }
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
fn anime_unknown_original_keeps_all_audio_and_warns() {
    let ja_en = audio(&[Some("ja"), Some("en")], None, true);
    assert_eq!(ja_en.kept_indices(), [0, 1]);
    assert_eq!(ja_en.warnings, ["original-language-unknown"]);
    assert!(!ja_en.needs_remux);

    let en_de = audio(&[Some("en"), Some("de")], None, true);
    assert_eq!(en_de.kept_indices(), [0, 1]);
    assert_eq!(en_de.warnings, ["original-language-unknown"]);
    assert!(!en_de.needs_remux);

    let no_audio = plan(
        &FileFacts {
            tracks: vec![
                track(0, TrackKind::Video, None),
                track(1, TrackKind::Subtitle, Some("en")),
            ],
            container: "matroska".into(),
            chapters: 0,
        },
        &TitleMeta {
            original: None,
            anime: true,
        },
    )
    .unwrap();
    assert_eq!(no_audio.kept_indices(), [0, 1]);
    assert!(no_audio.warnings.is_empty());
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
            chapters: 0,
        },
        &TitleMeta::default(),
    )
    .unwrap();
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
                chapters: 0,
            },
            &TitleMeta::default(),
        )
        .unwrap();
        assert_eq!(p.kept_indices(), [0]);
        assert!(p.warnings.is_empty());
        assert!(!p.needs_remux);
    }
    let p = plan(
        &FileFacts {
            tracks: vec![],
            container: "mkv".into(),
            chapters: 0,
        },
        &TitleMeta::default(),
    )
    .unwrap();
    assert!(!p.needs_remux);
}

#[test]
fn all_undetermined_and_invalid_original_rejected() {
    let tracks = vec![
        track(0, TrackKind::Audio, Some("und")),
        track(1, TrackKind::Subtitle, Some("")),
    ];
    let p = plan(
        &FileFacts {
            tracks,
            container: "mkv".into(),
            chapters: 0,
        },
        &TitleMeta {
            original: Some("en".into()),
            anime: false,
        },
    )
    .unwrap();
    assert_eq!(p.kept_indices(), [0, 1]);
    assert_eq!(p.warnings, ["language-undetermined"]);
    assert!(!p.needs_remux);

    for bad in ["xx", "english", "invalid", ""] {
        assert_eq!(
            try_audio(&[Some("en")], Some(bad), false),
            Err(Error::InvalidOriginalLanguage(bad.to_owned()))
        );
    }
}
