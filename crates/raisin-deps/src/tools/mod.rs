// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! External tool checkers.
//!
//! This module contains implementations of `DependencyChecker` for various
//! external tools that RaisinDB can optionally use.

mod tesseract;

pub use tesseract::TesseractChecker;

// Future checkers can be added here:
// mod ffmpeg;
// mod imagemagick;
// pub use ffmpeg::FfmpegChecker;
// pub use imagemagick::ImageMagickChecker;
