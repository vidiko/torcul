// SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
// SPDX-License-Identifier: EUPL-1.2
mod error;
mod inspect;
pub mod io;
mod mp4;
mod mp4_boxes;
mod mp4_movie;
mod packets;
mod remux;
mod verify;
pub use error::{Error, Result};
pub use inspect::probe;
pub use remux::{RemuxStats, remux};
pub use verify::{DURATION_TOLERANCE_US, VerifyReport, duration_within_tolerance, verify};
