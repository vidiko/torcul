// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "ffmpeg")]
    #[error("{operation}: {source}")]
    Ffmpeg {
        operation: &'static str,
        source: ffmpeg_next::Error,
    },
    #[error("{operation}: {source}")]
    Io {
        operation: &'static str,
        source: std::io::Error,
    },
    #[cfg(feature = "ffmpeg")]
    #[error("MP4 atom: {0}")]
    Atom(#[from] mp4_atom::Error),
    #[error("{0}")]
    Invalid(String),
}
pub type Result<T> = std::result::Result<T, Error>;
#[cfg(feature = "ffmpeg")]
pub(crate) fn av<T>(
    operation: &'static str,
    value: std::result::Result<T, ffmpeg_next::Error>,
) -> Result<T> {
    value.map_err(|source| Error::Ffmpeg { operation, source })
}
pub(crate) fn fs<T>(operation: &'static str, value: std::io::Result<T>) -> Result<T> {
    value.map_err(|source| Error::Io { operation, source })
}
