// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use serde::{Deserialize, Deserializer, de};
use std::collections::BTreeSet;

/// Deserializes a `BTreeSet` while refusing duplicate input elements.
pub fn unique_set<'de, D, T>(input: D) -> std::result::Result<BTreeSet<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Ord,
{
    let items = Vec::<T>::deserialize(input)?;
    let count = items.len();
    let set: BTreeSet<T> = items.into_iter().collect();
    if set.len() != count {
        return Err(de::Error::custom("duplicate item in a set"));
    }
    Ok(set)
}
