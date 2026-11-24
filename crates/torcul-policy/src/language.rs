// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use rust_iso639::{LanguageCode, from_code_1, from_code_2b, from_code_2t, from_code_3};

/// Canonical ISO 639 3-letter language code, with BCP47 region/script suffix removed.
pub fn normalize_language(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.split('-').any(str::is_empty) {
        return None;
    }
    let base = trimmed.split('-').next()?;
    let lang = from_code_1(base)
        .or_else(|| from_code_3(base))
        .or_else(|| from_code_2t(base))
        .or_else(|| from_code_2b(base))?;
    canonical(lang).map(str::to_owned)
}

fn canonical(lang: LanguageCode) -> Option<&'static str> {
    [lang.code_3, lang.code_2t, lang.code_2b, lang.code]
        .into_iter()
        .find(|code| !code.is_empty())
}
