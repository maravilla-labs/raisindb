//! Content and multimodal message types.
//!
//! Provides [`ContentPart`] and [`MessageContent`] for representing
//! text, images, and other multimodal content within messages.

use serde::{Deserialize, Serialize};

/// A content part in a multimodal message.
///
/// Follows the OpenAI message content format where each part has a type
/// and type-specific fields.
///
/// # Examples
///
/// ```rust
/// use raisin_ai::types::ContentPart;
///
/// // Text content
/// let text = ContentPart::Text { text: "Hello!".to_string() };
///
/// // Image content (base64 encoded)
/// let image = ContentPart::Image {
///     data: "base64-data-here".to_string(),
///     media_type: "image/jpeg".to_string(),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Text content
    Text {
        /// The text content
        text: String,
    },
    /// Image content (base64 encoded)
    Image {
        /// Base64-encoded image data
        data: String,
        /// MIME type (e.g., "image/jpeg", "image/png")
        media_type: String,
    },
    /// Image URL reference (for providers that support URLs)
    #[serde(rename = "image_url")]
    ImageUrl {
        /// The image URL
        url: String,
    },
}

impl ContentPart {
    /// Creates a text content part.
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text { text: text.into() }
    }

    /// Creates an image content part from base64 data.
    pub fn image(data: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentPart::Image {
            data: data.into(),
            media_type: media_type.into(),
        }
    }

    /// Creates an image URL content part.
    pub fn image_url(url: impl Into<String>) -> Self {
        ContentPart::ImageUrl { url: url.into() }
    }

    /// Returns the text content if this is a text part.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentPart::Text { text } => Some(text),
            _ => None,
        }
    }

    /// Returns true if this is an image part (base64 or URL).
    pub fn is_image(&self) -> bool {
        matches!(
            self,
            ContentPart::Image { .. } | ContentPart::ImageUrl { .. }
        )
    }

    /// Returns the image data if this is a base64 image part.
    pub fn as_image_data(&self) -> Option<(&str, &str)> {
        match self {
            ContentPart::Image { data, media_type } => Some((data, media_type)),
            _ => None,
        }
    }
}

/// Message content that can be either simple text or multimodal parts.
///
/// This follows the OpenAI message content format where `content` can be
/// either a string or an array of content parts.
///
/// # Examples
///
/// ```rust
/// use raisin_ai::types::{MessageContent, ContentPart};
///
/// // Simple text (most common)
/// let text = MessageContent::Text("Hello!".to_string());
///
/// // Multimodal with image
/// let multimodal = MessageContent::Parts(vec![
///     ContentPart::text("What's in this image?"),
///     ContentPart::image("base64-data", "image/jpeg"),
/// ]);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Simple text content
    Text(String),
    /// Array of content parts (for multimodal)
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    /// Extracts all text content concatenated together.
    ///
    /// For `Text`, returns the text.
    /// For `Parts`, concatenates all text parts with spaces.
    pub fn extract_text(&self) -> String {
        match self {
            MessageContent::Text(text) => text.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| p.as_text())
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Extracts the first image data found (base64, media_type).
    pub fn extract_first_image(&self) -> Option<(&str, &str)> {
        match self {
            MessageContent::Text(_) => None,
            MessageContent::Parts(parts) => parts.iter().find_map(|p| p.as_image_data()),
        }
    }

    /// Returns true if this content contains any images.
    pub fn has_images(&self) -> bool {
        match self {
            MessageContent::Text(_) => false,
            MessageContent::Parts(parts) => parts.iter().any(|p| p.is_image()),
        }
    }

    /// Creates simple text content.
    pub fn text(text: impl Into<String>) -> Self {
        MessageContent::Text(text.into())
    }

    /// Creates multimodal content from parts.
    pub fn parts(parts: Vec<ContentPart>) -> Self {
        MessageContent::Parts(parts)
    }
}

impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        MessageContent::Text(s)
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        MessageContent::Text(s.to_string())
    }
}
