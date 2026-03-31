// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Domain-specific value types: `RaisinReference`, `RaisinUrl`, and `Resource`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::property_value::{DateTimeTimestamp, PropertyValue};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
pub struct RaisinReference {
    #[serde(rename = "raisin:ref")]
    pub id: String,
    #[serde(rename = "raisin:workspace")]
    pub workspace: String,
    /// Path to the referenced node. Optional on input - will be auto-populated
    /// during INSERT/UPDATE if `id` starts with `/` (path-based reference).
    #[serde(
        rename = "raisin:path",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub path: String,
}

/// Rich URL type with optional metadata for previews, social sharing, etc.
///
/// All fields except `url` are optional - use what you need.
///
/// # JSON Examples
///
/// Minimal:
/// ```json
/// {"raisin:url": "https://example.com"}
/// ```
///
/// Rich link preview:
/// ```json
/// {
///   "raisin:url": "https://blog.example.com/post/123",
///   "raisin:title": "How to Build a Database",
///   "raisin:description": "A comprehensive guide...",
///   "raisin:image": "https://blog.example.com/og-image.jpg"
/// }
/// ```
///
/// Video embed:
/// ```json
/// {
///   "raisin:url": "https://youtube.com/watch?v=abc123",
///   "raisin:type": "video",
///   "raisin:embed": "https://youtube.com/embed/abc123",
///   "raisin:duration": 342
/// }
/// ```
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
pub struct RaisinUrl {
    // ===== REQUIRED =====
    /// The URL string (validated on deserialization)
    #[serde(rename = "raisin:url")]
    pub url: String,

    // ===== DISPLAY =====
    /// Display text for the link (e.g., "Click here")
    #[serde(rename = "raisin:label", skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Page title (from og:title or <title>)
    #[serde(rename = "raisin:title", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Page description (from og:description or meta description)
    #[serde(rename = "raisin:description", skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    // ===== MEDIA =====
    /// Favicon or icon URL
    #[serde(rename = "raisin:icon", skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Preview/thumbnail image URL (from og:image)
    #[serde(rename = "raisin:image", skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,

    // ===== METADATA =====
    /// Content type hint: "website", "video", "audio", "document", "image", "article"
    #[serde(rename = "raisin:type", skip_serializing_if = "Option::is_none")]
    pub link_type: Option<String>,

    /// Site name (from og:site_name)
    #[serde(rename = "raisin:site", skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,

    /// Author or creator name
    #[serde(rename = "raisin:author", skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    /// Published date (ISO 8601)
    #[serde(rename = "raisin:published", skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,

    // ===== BEHAVIOR =====
    /// Link target: "_blank", "_self", "_parent", "_top"
    #[serde(rename = "raisin:target", skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,

    /// Relationship hint: "nofollow", "sponsored", "ugc", "noopener"
    #[serde(rename = "raisin:rel", skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,

    // ===== VIDEO/AUDIO EMBED =====
    /// Embed URL (for video/audio players)
    #[serde(rename = "raisin:embed", skip_serializing_if = "Option::is_none")]
    pub embed_url: Option<String>,

    /// Duration in seconds (for video/audio)
    #[serde(rename = "raisin:duration", skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,

    /// Width in pixels (for embed/image)
    #[serde(rename = "raisin:width", skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,

    /// Height in pixels (for embed/image)
    #[serde(rename = "raisin:height", skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
}

impl RaisinUrl {
    /// Create a minimal RaisinUrl with just the URL
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            label: None,
            title: None,
            description: None,
            icon: None,
            image: None,
            link_type: None,
            site_name: None,
            author: None,
            published: None,
            target: None,
            rel: None,
            embed_url: None,
            duration: None,
            width: None,
            height: None,
        }
    }

    /// Parse and validate a URL string
    pub fn parse(url_str: &str) -> Result<Self, url::ParseError> {
        let parsed = url::Url::parse(url_str)?;
        Ok(Self::new(parsed.to_string()))
    }

    /// Set the display label
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the page title
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the preview image URL
    pub fn with_image(mut self, image: impl Into<String>) -> Self {
        self.image = Some(image.into());
        self
    }

    /// Set the link type (website, video, audio, document, image, article)
    pub fn with_type(mut self, link_type: impl Into<String>) -> Self {
        self.link_type = Some(link_type.into());
        self
    }

    /// Set the embed URL for video/audio
    pub fn with_embed(mut self, embed_url: impl Into<String>) -> Self {
        self.embed_url = Some(embed_url.into());
        self
    }

    /// Set target="_blank" for external links
    pub fn external(mut self) -> Self {
        self.target = Some("_blank".to_string());
        self.rel = Some("noopener".to_string());
        self
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
pub struct Resource {
    pub uuid: String,
    pub name: Option<String>,
    pub size: Option<i64>,
    pub mime_type: Option<String>,
    pub url: Option<String>,
    pub metadata: Option<HashMap<String, PropertyValue>>,
    pub is_loaded: Option<bool>,
    pub is_external: Option<bool>,
    pub created_at: DateTimeTimestamp,
    pub updated_at: DateTimeTimestamp,
}
