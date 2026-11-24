// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid ISO 639 original language: {0:?}")]
    InvalidOriginalLanguage(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
    Attachment,
    Data,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub index: usize,
    pub kind: TrackKind,
    /// FFmpeg codec descriptor name (uniquely identifies the codec id).
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub dispositions: BTreeSet<String>,
    pub attached_pic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFacts {
    pub tracks: Vec<TrackInfo>,
    pub container: String,
    #[serde(default)]
    pub chapters: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleMeta {
    pub original: Option<String>,
    pub anime: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackDecision {
    pub index: usize,
    pub keep: bool,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub decisions: Vec<TrackDecision>,
    pub warnings: Vec<String>,
    pub default_audio_change: Option<(usize, usize)>,
    pub needs_remux: bool,
}

impl Plan {
    pub fn kept_indices(&self) -> Vec<usize> {
        self.decisions
            .iter()
            .filter(|d| d.keep)
            .map(|d| d.index)
            .collect()
    }

    pub fn dropped_indices(&self) -> Vec<usize> {
        self.decisions
            .iter()
            .filter(|d| !d.keep)
            .map(|d| d.index)
            .collect()
    }
}
