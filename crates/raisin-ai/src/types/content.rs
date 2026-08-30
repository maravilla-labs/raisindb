//! Content and multimodal message types.
//!
//! Provides [`ContentPart`] and [`MessageContent`] for representing
//! text, images, and other multimodal content within messages.

use serde::{Deserialize, Deserializer, Serialize};

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
#[derive(Debug, Clone, Serialize)]
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

    /// Creates an image part from a URL, collapsing a base64 `data:` URL into
    /// the inline [`ContentPart::Image`] form.
    ///
    /// This is the single place a `data:` URL is taken apart. A function that
    /// reads an asset's bytes out of RaisinDB and base64s them writes a `data:`
    /// URL, and every provider that can carry an image wants the two halves
    /// separately — so doing it once here is what keeps six providers from each
    /// growing their own parser.
    pub fn from_url(url: impl AsRef<str>) -> Self {
        let url = url.as_ref();
        match split_data_url(url) {
            Some((data, media_type)) => ContentPart::image(data, media_type),
            None => ContentPart::image_url(url),
        }
    }

    /// Returns the remote image URL if this is a URL-referenced image part.
    pub fn as_image_url(&self) -> Option<&str> {
        match self {
            ContentPart::ImageUrl { url } => Some(url),
            _ => None,
        }
    }

    /// Renders this part as a `data:` URL when it holds inline base64 image
    /// bytes, or returns the remote URL as-is.
    ///
    /// The OpenAI-shaped providers want exactly this string, so they do not
    /// have to reassemble it themselves.
    pub fn as_url(&self) -> Option<String> {
        match self {
            ContentPart::Image { data, media_type } => {
                Some(format!("data:{media_type};base64,{data}"))
            }
            ContentPart::ImageUrl { url } => Some(url.clone()),
            ContentPart::Text { .. } => None,
        }
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

/// Every spelling of a content part this deserializer accepts, and what each
/// one normalizes to.
///
/// # Why this is hand-written
///
/// The derived `#[serde(tag = "type")]` implementation accepted exactly ONE
/// image spelling — `{"type": "image_url", "url": "…"}` — which no client
/// anywhere actually writes. A function author following the OpenAI format
/// (`{"type": "image_url", "image_url": {"url": "…"}}`), the OpenAI *Responses*
/// format (`input_image`), or the Anthropic format (`source: {…}`) got a
/// deserialization error from deep inside `CompletionRequest`, phrased in terms
/// of a Rust enum they cannot see. So "multimodal is supported" was true of the
/// types and false of everything that could reach them.
///
/// # Why the normalization happens HERE and not per provider
///
/// A `data:` URL and a base64 blob with a separate media type are the same
/// thing written two ways, and a function that reads bytes out of RaisinDB and
/// base64s them produces the `data:` form. Normalizing at the edge means every
/// provider downstream handles exactly TWO cases — inline base64, or a real
/// remote URL — instead of each one growing its own `data:` parser. This
/// codebase's dominant bug class is mirrored paths that drift; six independent
/// data-URL splitters is that bug waiting to happen.
///
/// Accepted, and the canonical variant each becomes:
///
/// | written as | becomes |
/// |---|---|
/// | `{type: "text", text}` | `Text` |
/// | `{type: "input_text", text}` (OpenAI Responses) | `Text` |
/// | `{type: "image", data, media_type}` (native) | `Image` |
/// | `{type: "image", source: {type: "base64", media_type, data}}` (Anthropic) | `Image` |
/// | `{type: "image", source: {type: "url", url}}` | `Image` if `data:`, else `ImageUrl` |
/// | `{type: "image_url", image_url: {url}}` (OpenAI chat) | `Image` if `data:`, else `ImageUrl` |
/// | `{type: "image_url", image_url: "…"}` | ditto |
/// | `{type: "image_url", url}` (the one form that worked before) | ditto |
/// | `{type: "input_image", image_url}` (OpenAI Responses) | ditto |
impl<'de> Deserialize<'de> for ContentPart {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error as DeError;

        let raw = serde_json::Value::deserialize(deserializer)?;
        let obj = raw
            .as_object()
            .ok_or_else(|| D::Error::custom("a content part must be an object"))?;

        let part_type = obj
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| D::Error::custom("a content part must carry a `type`"))?;

        match part_type {
            "text" | "input_text" => {
                let text = obj
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| D::Error::custom("a text content part must carry `text`"))?;
                Ok(ContentPart::text(text))
            }
            "image" | "image_url" | "input_image" => {
                // The flat native form first: it is unambiguous and needs no
                // URL parsing at all.
                if let (Some(data), Some(media_type)) = (
                    obj.get("data").and_then(|v| v.as_str()),
                    obj.get("media_type").and_then(|v| v.as_str()),
                ) {
                    return Ok(ContentPart::image(data, media_type));
                }

                // Anthropic's `source` object, in both of its shapes.
                if let Some(source) = obj.get("source").and_then(|v| v.as_object()) {
                    if let (Some(data), Some(media_type)) = (
                        source.get("data").and_then(|v| v.as_str()),
                        source.get("media_type").and_then(|v| v.as_str()),
                    ) {
                        return Ok(ContentPart::image(data, media_type));
                    }
                    if let Some(url) = source.get("url").and_then(|v| v.as_str()) {
                        return Ok(ContentPart::from_url(url));
                    }
                }

                // Everything else resolves to a single URL string, whether it
                // was written bare or wrapped in an `image_url` object.
                let url = obj
                    .get("image_url")
                    .and_then(|v| v.as_str().or_else(|| v.get("url").and_then(|u| u.as_str())))
                    .or_else(|| obj.get("url").and_then(|v| v.as_str()))
                    .ok_or_else(|| {
                        D::Error::custom(
                            "an image content part must carry one of: \
                             `data` + `media_type`, `source`, `image_url`, or `url`",
                        )
                    })?;
                Ok(ContentPart::from_url(url))
            }
            other => Err(D::Error::custom(format!(
                "unknown content part type `{other}`; expected one of: \
                 text, input_text, image, image_url, input_image"
            ))),
        }
    }
}

/// Splits `data:<media-type>;base64,<payload>` into its two halves.
///
/// Returns `None` for anything that is not a base64 `data:` URL, including a
/// `data:` URL that is percent-encoded rather than base64 — carrying that one
/// through as a remote URL fails loudly at the provider instead of silently
/// sending a provider a payload it will reject as an image.
fn split_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let media_type = meta.strip_suffix(";base64")?;
    if media_type.is_empty() || payload.is_empty() {
        return None;
    }
    Some((payload.to_string(), media_type.to_string()))
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
