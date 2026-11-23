// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TitleMeta {
    pub original: Option<String>,
    pub anime: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackDecision {
    pub index: usize,
    pub keep: bool,
    pub reason: String,
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
}
