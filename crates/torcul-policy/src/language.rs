// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use isolang::Language;

/// Canonical ISO 639-3 language, with BCP47 region/script suffix removed.
pub fn normalize_language(value: &str) -> Option<String> {
    let lower = value.trim().split('-').next()?.to_ascii_lowercase();
    let canonical = bibliographic_alias(&lower);
    Language::from_639_1(canonical)
        .or_else(|| Language::from_639_3(canonical))
        .map(|lang| lang.to_639_3().to_owned())
}

// ISO 639-2/B aliases; all other codes resolved by isolang's maintained data.
// Reference: https://www.loc.gov/standards/iso639-2/php/code_list.php
fn bibliographic_alias(code: &str) -> &str {
    const ALIASES: [(&str, &str); 20] = [
        ("alb", "sqi"),
        ("arm", "hye"),
        ("baq", "eus"),
        ("bur", "mya"),
        ("chi", "zho"),
        ("cze", "ces"),
        ("dut", "nld"),
        ("fre", "fra"),
        ("geo", "kat"),
        ("ger", "deu"),
        ("gre", "ell"),
        ("ice", "isl"),
        ("mac", "mkd"),
        ("mao", "mri"),
        ("may", "msa"),
        ("per", "fas"),
        ("rum", "ron"),
        ("slo", "slk"),
        ("tib", "bod"),
        ("wel", "cym"),
    ];
    ALIASES
        .iter()
        .find(|(from, _)| *from == code)
        .map_or(code, |(_, to)| *to)
}
