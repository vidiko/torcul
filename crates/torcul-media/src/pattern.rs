// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use regex::Regex;
use serde::de::Error;

pub const FINGERPRINT_SHA256: &str = r"^sha256:[0-9a-f]{64}$";
pub const PACKET_SHA256: &str = r"^[0-9a-f]{64}$";

pub fn matching<E: Error>(value: String, pattern: &str, what: &str) -> Result<String, E> {
    let compiled = Regex::new(pattern)
        .map_err(|err| E::custom(format!("invalid regex {pattern:?}: {err}")))?;
    if !compiled.is_match(&value) {
        return Err(E::custom(format!(
            "{what} {value:?} does not match {pattern}"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::value::Error as ValueError;

    #[test]
    fn matching_accepts_valid_and_rejects_mismatch_or_bad_pattern() {
        let valid_fp =
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned();
        assert_eq!(
            matching::<ValueError>(valid_fp.clone(), FINGERPRINT_SHA256, "fingerprint sha256")
                .unwrap(),
            valid_fp
        );

        let mismatch =
            matching::<ValueError>("short".to_owned(), PACKET_SHA256, "packet sha256").unwrap_err();
        assert!(mismatch.to_string().contains("packet sha256"));
        assert!(mismatch.to_string().contains(PACKET_SHA256));

        let bad_re = matching::<ValueError>("abc".to_owned(), "(", "sample").unwrap_err();
        assert!(bad_re.to_string().contains("\"(\""));
    }
}
