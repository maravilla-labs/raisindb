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

//! [`ContentBlock`] — one entry of a tool result or resource read.
//!
//! The codec is hand-written rather than derived. An internally-tagged derive
//! rejects any `type` it does not know, and this enum is now parsed in BOTH
//! directions: RaisinDB emits it as a server, and [`crate::client`] parses it
//! out of arbitrary remote servers, which ship block types this crate has never
//! heard of. `serde(other)` is not a fix — it only accepts a UNIT variant, so
//! the block's payload would be silently discarded and a tool result would
//! quietly lose content. [`ContentBlock::Other`] keeps the raw JSON instead, and
//! round-trips it verbatim on the way out.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};

/// A single content block in a tool result or resource read.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentBlock {
    /// Plain-text content.
    Text {
        /// The text body.
        text: String,
    },
    /// Structured JSON content.
    ///
    /// A RaisinDB extension, not a spec block type — strict clients reject it,
    /// which is why [`crate::protocol::CallToolResult::json`] serializes through
    /// a `text` block instead.
    Json {
        /// The JSON value.
        json: Value,
    },
    /// An embedded resource (e.g. an MCP-UI widget delivered inline as
    /// `text/html`, or a `text/uri-list` pointer to an iframable page).
    Resource {
        /// The embedded resource contents.
        resource: crate::resource_types::ResourceContents,
    },
    /// Base64 image data.
    Image {
        /// Base64-encoded image bytes.
        data: String,
        /// Mime type of the encoded bytes.
        mime_type: String,
    },
    /// Base64 audio data.
    Audio {
        /// Base64-encoded audio bytes.
        data: String,
        /// Mime type of the encoded bytes.
        mime_type: String,
    },
    /// A pointer to a resource the caller may read separately.
    ResourceLink {
        /// URI of the linked resource.
        uri: String,
        /// Optional human-readable name.
        name: Option<String>,
        /// Optional mime type hint.
        mime_type: Option<String>,
    },
    /// A block type this crate does not model, preserved verbatim.
    Other {
        /// The block's `type` discriminant (empty when it had none).
        block_type: String,
        /// The complete original JSON object.
        raw: Value,
    },
}

impl ContentBlock {
    /// Build a text content block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// Build a JSON content block.
    pub fn json(json: Value) -> Self {
        Self::Json { json }
    }

    /// Build an embedded-resource content block.
    pub fn resource(resource: crate::resource_types::ResourceContents) -> Self {
        Self::Resource { resource }
    }

    /// Build a spec-compliant `text` content block holding a serialized JSON
    /// value (pretty-printed for readability; compact on serialize failure).
    pub fn json_text(value: &Value) -> Self {
        Self::Text {
            text: serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
        }
    }

    /// The block's wire `type` discriminant.
    pub fn block_type(&self) -> &str {
        match self {
            Self::Text { .. } => "text",
            Self::Json { .. } => "json",
            Self::Resource { .. } => "resource",
            Self::Image { .. } => "image",
            Self::Audio { .. } => "audio",
            Self::ResourceLink { .. } => "resource_link",
            Self::Other { block_type, .. } => block_type,
        }
    }

    /// The text body, for the block types that carry one.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }
}

impl Serialize for ContentBlock {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = match self {
            Self::Text { text } => json!({ "type": "text", "text": text }),
            Self::Json { json } => json!({ "type": "json", "json": json }),
            Self::Resource { resource } => {
                let encoded = serde_json::to_value(resource).map_err(serde::ser::Error::custom)?;
                json!({ "type": "resource", "resource": encoded })
            }
            Self::Image { data, mime_type } => {
                json!({ "type": "image", "data": data, "mimeType": mime_type })
            }
            Self::Audio { data, mime_type } => {
                json!({ "type": "audio", "data": data, "mimeType": mime_type })
            }
            Self::ResourceLink {
                uri,
                name,
                mime_type,
            } => {
                let mut block = json!({ "type": "resource_link", "uri": uri });
                if let Some(name) = name {
                    block["name"] = json!(name);
                }
                if let Some(mime_type) = mime_type {
                    block["mimeType"] = json!(mime_type);
                }
                block
            }
            // Round-trip verbatim: whatever the peer sent goes back out
            // unchanged, including fields this crate never interpreted.
            Self::Other { raw, .. } => raw.clone(),
        };
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ContentBlock {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let block_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        // A known `type` with a malformed body is an error, not an `Other`:
        // silently degrading it would hide a real protocol mismatch behind a
        // block the caller cannot interpret either.
        let field_str = |key: &str| -> Result<String, D::Error> {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    serde::de::Error::custom(format!(
                        "content block of type `{block_type}` is missing string field `{key}`"
                    ))
                })
        };
        let opt_str = |key: &str| -> Option<String> {
            value.get(key).and_then(Value::as_str).map(str::to_string)
        };

        match block_type.as_str() {
            "text" => Ok(Self::Text {
                text: field_str("text")?,
            }),
            "json" => Ok(Self::Json {
                json: value.get("json").cloned().unwrap_or(Value::Null),
            }),
            "resource" => {
                let resource = value.get("resource").cloned().ok_or_else(|| {
                    serde::de::Error::custom(
                        "content block of type `resource` is missing `resource`",
                    )
                })?;
                Ok(Self::Resource {
                    resource: serde_json::from_value(resource).map_err(serde::de::Error::custom)?,
                })
            }
            "image" => Ok(Self::Image {
                data: field_str("data")?,
                mime_type: field_str("mimeType")?,
            }),
            "audio" => Ok(Self::Audio {
                data: field_str("data")?,
                mime_type: field_str("mimeType")?,
            }),
            "resource_link" => Ok(Self::ResourceLink {
                uri: field_str("uri")?,
                name: opt_str("name"),
                mime_type: opt_str("mimeType"),
            }),
            _ => Ok(Self::Other {
                block_type,
                raw: value,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_spec_block_types() {
        let blocks: Vec<ContentBlock> = serde_json::from_value(json!([
            { "type": "text", "text": "hello" },
            { "type": "image", "data": "aGk=", "mimeType": "image/png" },
            { "type": "audio", "data": "aGk=", "mimeType": "audio/wav" },
            { "type": "resource_link", "uri": "file:///a", "name": "a", "mimeType": "text/plain" },
        ]))
        .expect("spec block types must parse");

        assert_eq!(blocks[0], ContentBlock::text("hello"));
        assert_eq!(blocks[1].block_type(), "image");
        assert_eq!(blocks[2].block_type(), "audio");
        assert_eq!(blocks[3].block_type(), "resource_link");
    }

    #[test]
    fn unknown_block_is_preserved_not_dropped() {
        let raw = json!({ "type": "video", "url": "https://x/y.mp4", "durationMs": 12 });
        let block: ContentBlock = serde_json::from_value(raw.clone()).unwrap();

        assert_eq!(block.block_type(), "video");
        // The whole point: an unmodelled block round-trips with every field
        // intact rather than collapsing to a payload-free marker.
        assert_eq!(serde_json::to_value(&block).unwrap(), raw);
    }

    #[test]
    fn known_type_with_bad_body_is_an_error() {
        let err =
            serde_json::from_value::<ContentBlock>(json!({ "type": "image", "data": "aGk=" }));
        assert!(
            err.is_err(),
            "a malformed image block must not degrade to Other"
        );
    }

    #[test]
    fn spec_block_types_round_trip() {
        for block in [
            ContentBlock::text("hi"),
            ContentBlock::Image {
                data: "aGk=".into(),
                mime_type: "image/png".into(),
            },
            ContentBlock::ResourceLink {
                uri: "file:///a".into(),
                name: None,
                mime_type: None,
            },
        ] {
            let encoded = serde_json::to_value(&block).unwrap();
            let decoded: ContentBlock = serde_json::from_value(encoded).unwrap();
            assert_eq!(block, decoded);
        }
    }
}
