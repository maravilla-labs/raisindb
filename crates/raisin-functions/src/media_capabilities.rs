// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! What this server can actually do with a binary asset, per media kind.
//!
//! ## Why this exists
//!
//! Core's binary handling is much narrower than the estate assumes. The whole
//! text-extraction vocabulary is one mimetype — `is_extractable_mime` in
//! `raisin-rocksdb/src/jobs/handlers/asset_processing/helpers.rs` matches
//! `application/pdf` and nothing else — while Word, PowerPoint, Excel, video
//! and audio are handled only by an out-of-tree plugin that exists on the
//! Studio installation. That is a deliberate split, not a gap: LibreOffice,
//! ffmpeg, ImageMagick and headless Chrome must not live inside the raisindb
//! process.
//!
//! What was missing is the ability to ASK. Nothing exposed the plugin registry,
//! so a pipeline could not say "plugin if present, else core, else record
//! unsupported" — and a missing plugin was indistinguishable from "nothing to
//! do". This module is the one place that answers, and it is read by three
//! consumers so they cannot drift: the startup capability report, the JS probe
//! (`raisin.capabilities`, assembled in [`crate::plugin`]) and any management
//! endpoint.
//!
//! ## What counts as evidence
//!
//! The `core` column below is derived from code, never from a feature name:
//!
//! * **PDF text + OCR** — `raisin-functions` depends on `raisin-ai` with
//!   `features = ["pdf", "pdf-markdown", "ocr"]` unconditionally
//!   (`crates/raisin-functions/Cargo.toml`), surfaced as `raisin.pdf.*`
//!   (`runtime/quickjs/api_wrapper.js`) and `resource.processDocument()`.
//! * **PDF asset job** — the deprecated `AssetProcessing` handler, whose
//!   dispatch is `is_extractable_mime` → `application/pdf` only.
//! * **Image embedding + OCR** — CLIP via candle and tesseract in the same
//!   asset handler (`is_image_mime` + `generate_image_embedding`). Captioning
//!   is compiled in but explicitly disabled (`log_deprecated_captioning_warning`).
//! * **Office, video, audio** — no code at all. Grepping the workspace for
//!   `docx|pptx|xlsx|officedocument|odt|libreoffice` returns only virtual-mount
//!   test fixtures.

use serde::Serialize;

use crate::plugin::{plugin_manifest, rejected_plugins};

/// What core itself can do with a media kind, in this binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreSupport {
    /// Core opens the bytes and produces text (with OCR fallback where noted).
    TextExtraction,
    /// Core produces a CLIP embedding and can OCR the image. No resize, no
    /// thumbnail, no face detection.
    ImageEmbeddingAndOcr,
    /// Core never opens these bytes. The same content pasted into a node
    /// *property* is embedded normally; as an uploaded asset it is inert.
    None,
}

impl CoreSupport {
    fn describe(self) -> &'static str {
        match self {
            CoreSupport::TextExtraction => "text extraction (native + OCR fallback)",
            CoreSupport::ImageEmbeddingAndOcr => "CLIP embedding + OCR",
            CoreSupport::None => "nothing",
        }
    }
}

/// One media kind: what core does, and which plugin method would do more.
#[derive(Clone, Copy, Debug)]
pub struct MediaKind {
    /// Short label used in the startup report (`docx`, `pdf`, `mp4`…).
    pub label: &'static str,
    /// Mimetypes this row covers.
    pub mime_types: &'static [&'static str],
    /// What core does with it, in THIS binary.
    pub core: CoreSupport,
    /// A representative plugin method that would handle the kind. Presence of
    /// this method in the registry is what makes the kind "plugin-backed".
    /// Empty when no plugin in the estate handles it either.
    pub plugin_method: Option<&'static str>,
}

/// The one table. Adding a media kind means adding a row here, not a second
/// mimetype list somewhere else.
pub const MEDIA_KINDS: &[MediaKind] = &[
    MediaKind {
        label: "pdf",
        mime_types: &["application/pdf"],
        core: CoreSupport::TextExtraction,
        plugin_method: Some("media.doc.thumbnail"),
    },
    MediaKind {
        label: "docx",
        mime_types: &[
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/msword",
        ],
        core: CoreSupport::None,
        plugin_method: Some("media.doc.toPdf"),
    },
    MediaKind {
        label: "pptx",
        mime_types: &[
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/vnd.ms-powerpoint",
        ],
        core: CoreSupport::None,
        plugin_method: Some("media.doc.toPdf"),
    },
    MediaKind {
        label: "xlsx",
        mime_types: &[
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.ms-excel",
        ],
        core: CoreSupport::None,
        plugin_method: Some("media.doc.convert"),
    },
    MediaKind {
        label: "odt",
        mime_types: &["application/vnd.oasis.opendocument.text"],
        core: CoreSupport::None,
        plugin_method: Some("media.doc.convert"),
    },
    MediaKind {
        label: "image",
        mime_types: &["image/png", "image/jpeg", "image/tiff", "image/webp"],
        core: CoreSupport::ImageEmbeddingAndOcr,
        plugin_method: Some("media.image.resize"),
    },
    MediaKind {
        label: "video",
        mime_types: &["video/mp4", "video/quicktime", "video/webm"],
        core: CoreSupport::None,
        plugin_method: Some("media.video.transcode"),
    },
    MediaKind {
        label: "audio",
        mime_types: &["audio/mpeg", "audio/mp4", "audio/wav"],
        core: CoreSupport::None,
        plugin_method: Some("media.video.extractAudio"),
    },
    MediaKind {
        label: "text",
        mime_types: &["text/plain", "text/markdown", "text/csv"],
        core: CoreSupport::None,
        plugin_method: None,
    },
    MediaKind {
        label: "html",
        mime_types: &["text/html"],
        core: CoreSupport::None,
        plugin_method: Some("media.browser.extractHtml"),
    },
];

/// Who handles a media kind on this server, right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum Provider {
    /// A loaded plugin services the kind's method.
    Plugin { plugin: String, method: String },
    /// No plugin, but core handles it in-process.
    Core { how: &'static str },
    /// Nobody handles it. A pipeline that meets these bytes MUST record an
    /// `unsupported` trace rather than returning quietly — see
    /// `CORE-PLUGIN-DEPLOYMENT.md` §2.
    Unsupported,
}

/// One resolved row of the capability report.
#[derive(Clone, Debug, Serialize)]
pub struct CapabilityRow {
    pub kind: &'static str,
    pub mime_types: Vec<&'static str>,
    #[serde(flatten)]
    pub provider: Provider,
}

impl CapabilityRow {
    /// The startup-log rendering: `docx: plugin OK (maravilla-media)`.
    pub fn render(&self) -> String {
        match &self.provider {
            Provider::Plugin { plugin, .. } => format!("{}: plugin OK ({plugin})", self.kind),
            Provider::Core { how } => format!("{}: core ({how})", self.kind),
            Provider::Unsupported => format!("{}: UNSUPPORTED", self.kind),
        }
    }
}

/// Resolve every media kind against the plugins registered in this process.
///
/// Cheap and pure; the registry is only written at startup, before any function
/// runs, so a caller may cache the result for the process lifetime. It cannot
/// change without a restart — the plugin directory is scanned exactly once
/// (`load_plugins_from_dir`).
pub fn capability_report() -> Vec<CapabilityRow> {
    let manifest = plugin_manifest();
    MEDIA_KINDS
        .iter()
        .map(|kind| {
            let provider = kind
                .plugin_method
                .and_then(|method| {
                    manifest
                        .iter()
                        .find(|entry| entry.methods.iter().any(|m| m == method))
                        .map(|entry| Provider::Plugin {
                            plugin: entry.name.clone(),
                            method: method.to_string(),
                        })
                })
                .unwrap_or(match kind.core {
                    CoreSupport::None => Provider::Unsupported,
                    other => Provider::Core {
                        how: other.describe(),
                    },
                });
            CapabilityRow {
                kind: kind.label,
                mime_types: kind.mime_types.to_vec(),
                provider,
            }
        })
        .collect()
}

/// Log the capability report at startup, one line per media kind, plus every
/// plugin file the loader refused.
///
/// The point is that an operator sees a gap on BOOT — "docx: UNSUPPORTED" —
/// instead of discovering it when a customer's search comes back empty weeks
/// later. A rejected plugin is logged at WARN with its reason, because the
/// failure it produces (every `raisin.<ns>.*` call soft-failing estate-wide) is
/// otherwise invisible: the server boots green.
pub fn log_capability_report() {
    let manifest = plugin_manifest();
    let rejected = rejected_plugins();

    if manifest.is_empty() {
        tracing::info!(
            "media capabilities: no function plugins loaded (core-only build); \
             raisin.media.* returns {{ ok:false, reason:\"plugin_absent\" }}"
        );
    } else {
        for entry in &manifest {
            tracing::info!(
                plugin = %entry.name,
                methods = entry.methods.len(),
                "function plugin loaded"
            );
        }
    }

    for row in capability_report() {
        match row.provider {
            Provider::Unsupported => tracing::warn!("media capability {}", row.render()),
            _ => tracing::info!("media capability {}", row.render()),
        }
    }

    for rejection in &rejected {
        tracing::warn!(
            path = %rejection.path,
            reason = %rejection.reason,
            "plugin REJECTED — its bindings are absent for every tenant on this server"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_resolves_and_core_only_matches_the_code() {
        let rows = capability_report();
        assert_eq!(rows.len(), MEDIA_KINDS.len());

        // With no plugin registered (the unit-test process), the report must be
        // exactly what core's own code supports: PDF text and images. Anything
        // else claiming Core here would be this file lying about the binary.
        let core_kinds: Vec<&str> = rows
            .iter()
            .filter(|r| matches!(r.provider, Provider::Core { .. }))
            .map(|r| r.kind)
            .collect();
        if plugin_manifest().is_empty() {
            assert_eq!(core_kinds, vec!["pdf", "image"]);
            assert!(rows
                .iter()
                .any(|r| r.kind == "docx" && r.provider == Provider::Unsupported));
        }
    }

    #[test]
    fn rendering_names_the_provider() {
        for row in capability_report() {
            let line = row.render();
            assert!(line.starts_with(row.kind));
            match row.provider {
                Provider::Unsupported => assert!(line.contains("UNSUPPORTED")),
                Provider::Core { .. } => assert!(line.contains("core")),
                Provider::Plugin { .. } => assert!(line.contains("plugin OK")),
            }
        }
    }
}
