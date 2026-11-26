// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod language;
mod sets;
mod types;
pub use language::normalize_language;
pub use sets::unique_set;
use std::collections::BTreeSet;
pub use types::*;

#[cfg(test)]
mod tests;

/// Pure policy: decisions preserve input order; warnings are unique and sorted.
pub fn plan(facts: &FileFacts, meta: &TitleMeta) -> Result<Plan> {
    let original = match meta.original.as_deref() {
        Some(raw) => Some(
            normalize_language(raw)
                .ok_or_else(|| Error::InvalidOriginalLanguage(raw.to_owned()))?,
        ),
        None => None,
    };
    let mut warnings = BTreeSet::new();
    let audio: Vec<_> = facts
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .collect();
    let preferred = audio
        .iter()
        .any(|t| preferred_language(t.language.as_deref()));
    let fallback = audio_fallback(&audio, original.as_deref(), preferred, meta, &mut warnings);
    let decisions: Vec<_> = facts
        .tracks
        .iter()
        .map(|t| {
            let (keep, reason) = decision(t, original.as_deref(), preferred, fallback, meta.anime);
            if matches!(t.kind, TrackKind::Audio | TrackKind::Subtitle)
                && undetermined(t.language.as_deref())
            {
                warnings.insert("language-undetermined".to_owned());
            }
            TrackDecision {
                index: t.index,
                keep,
                reason: reason.to_owned(),
                lid: (!keep).then(|| "pending".to_owned()),
            }
        })
        .collect();
    let default_audio_change = moved_default(&audio, &decisions);
    if default_audio_change.is_some() {
        warnings.insert("default-audio-moved".to_owned());
    }
    Ok(Plan {
        needs_remux: decisions.iter().any(|d| !d.keep) || default_audio_change.is_some(),
        decisions,
        warnings: warnings.into_iter().collect(),
        default_audio_change,
    })
}

fn audio_fallback(
    audio: &[&TrackInfo],
    original: Option<&str>,
    preferred: bool,
    meta: &TitleMeta,
    warnings: &mut BTreeSet<String>,
) -> bool {
    if audio.is_empty() {
        return false;
    }
    if original.is_none() && (meta.anime || !preferred) {
        warnings.insert("original-language-unknown".to_owned());
        return true;
    }
    if preferred {
        return false;
    }
    let any_match = audio.iter().any(|t| {
        undetermined(t.language.as_deref())
            || original.is_some_and(|o| {
                t.language
                    .as_deref()
                    .and_then(normalize_language)
                    .as_deref()
                    == Some(o)
            })
    });
    if !any_match {
        warnings.insert("no-audio-matched".to_owned());
    }
    !any_match
}

fn decision(
    t: &TrackInfo,
    original: Option<&str>,
    preferred: bool,
    fallback: bool,
    anime: bool,
) -> (bool, &'static str) {
    match t.kind {
        TrackKind::Audio | TrackKind::Subtitle if undetermined(t.language.as_deref()) => {
            (true, "language-undetermined")
        }
        TrackKind::Audio if fallback => (true, "audio-fallback"),
        TrackKind::Audio if preferred_language(t.language.as_deref()) => {
            (true, "preferred-language")
        }
        TrackKind::Audio
            if (anime || !preferred)
                && original.is_some_and(|o| {
                    t.language
                        .as_deref()
                        .and_then(normalize_language)
                        .as_deref()
                        == Some(o)
                }) =>
        {
            (true, "original-language")
        }
        TrackKind::Subtitle if preferred_language(t.language.as_deref()) => {
            (true, "preferred-language")
        }
        TrackKind::Audio | TrackKind::Subtitle => (false, "language-not-selected"),
        _ => (true, "preserve-stream"),
    }
}

fn moved_default(audio: &[&TrackInfo], decisions: &[TrackDecision]) -> Option<(usize, usize)> {
    let dropped = audio.iter().find(|t| {
        t.dispositions.contains("default")
            && decisions.iter().any(|d| d.index == t.index && !d.keep)
    })?;
    let kept = audio
        .iter()
        .find(|t| decisions.iter().any(|d| d.index == t.index && d.keep))?;
    Some((dropped.index, kept.index))
}

fn preferred_language(lang: Option<&str>) -> bool {
    matches!(
        lang.and_then(normalize_language).as_deref(),
        Some("eng" | "deu")
    )
}

fn undetermined(lang: Option<&str>) -> bool {
    lang.is_none_or(|s| {
        let s = s.trim();
        s.is_empty() || s.eq_ignore_ascii_case("und")
    })
}
