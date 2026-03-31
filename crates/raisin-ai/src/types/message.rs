//! Message and role types for AI conversations.
//!
//! Contains [`Message`] with custom OpenAI-compatible deserialization
//! and the [`Role`] enum.

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use super::content::{ContentPart, MessageContent};
use super::tools::ToolCall;

/// A message in a conversation.
///
/// Supports both simple text content (via `content` field) and multimodal
/// content (via `content_parts` field) for vision models.
///
/// ## OpenAI-Compatible Deserialization
///
/// This struct uses custom deserialization to support OpenAI-style multimodal messages
/// where `content` can be either a string or an array of content parts:
///
/// ```json
/// // String content (text-only)
/// { "role": "user", "content": "Hello!" }
///
/// // Array content (multimodal)
/// { "role": "user", "content": [
///     { "type": "text", "text": "What's in this image?" },
///     { "type": "image", "data": "base64...", "media_type": "image/jpeg" }
/// ]}
/// ```
///
/// # Examples
///
/// ```rust
/// use raisin_ai::types::{Message, MessageContent, ContentPart};
///
/// // Simple text message
/// let text_msg = Message::user("Hello!");
///
/// // Multimodal message with image
/// let vision_msg = Message::user_multimodal(vec![
///     ContentPart::text("What's in this image?"),
///     ContentPart::image("base64-data", "image/jpeg"),
/// ]);
/// ```
#[derive(Debug, Clone, Serialize, Default)]
pub struct Message {
    /// The role of the message sender
    #[serde(default)]
    pub role: Role,

    /// The text content of the message (for simple text-only messages)
    #[serde(default)]
    pub content: String,

    /// Multimodal content parts (for messages with images, etc.)
    ///
    /// When present, this takes precedence over `content` for multimodal models.
    /// For backward compatibility, `content` is still serialized for text-only messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_parts: Option<MessageContent>,

    /// Optional tool calls made by the assistant
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Optional tool call ID (for tool responses)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    /// Optional name (for function/tool messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    /// Creates a new user message.
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_ai::types::Message;
    ///
    /// let msg = Message::user("Hello!");
    /// ```
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Creates a new user message with multimodal content.
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_ai::types::{Message, ContentPart};
    ///
    /// let msg = Message::user_multimodal(vec![
    ///     ContentPart::text("What's in this image?"),
    ///     ContentPart::image("base64-data", "image/jpeg"),
    /// ]);
    /// ```
    pub fn user_multimodal(parts: Vec<ContentPart>) -> Self {
        let text = parts
            .iter()
            .filter_map(|p| p.as_text())
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            role: Role::User,
            content: text,
            content_parts: Some(MessageContent::Parts(parts)),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Creates a new assistant message.
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_ai::types::Message;
    ///
    /// let msg = Message::assistant("Hi there!");
    /// ```
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Creates a new system message.
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_ai::types::Message;
    ///
    /// let msg = Message::system("You are a helpful assistant.");
    /// ```
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            content_parts: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    /// Creates a new tool response message.
    pub fn tool(content: impl Into<String>, tool_call_id: String, name: Option<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            content_parts: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
            name,
        }
    }

    /// Adds tool calls to an assistant message.
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = Some(tool_calls);
        self
    }

    /// Returns the effective text content of this message.
    ///
    /// If `content_parts` is present, extracts and concatenates text from parts.
    /// Otherwise, returns the `content` field.
    pub fn effective_text(&self) -> String {
        if let Some(ref parts) = self.content_parts {
            parts.extract_text()
        } else {
            self.content.clone()
        }
    }

    /// Returns the first image data in this message, if any.
    ///
    /// Returns `(base64_data, media_type)` if an image is found.
    pub fn first_image(&self) -> Option<(&str, &str)> {
        self.content_parts.as_ref()?.extract_first_image()
    }

    /// Returns true if this message contains image content.
    pub fn has_images(&self) -> bool {
        self.content_parts
            .as_ref()
            .is_some_and(|parts| parts.has_images())
    }

    /// Returns true if this is a multimodal message.
    pub fn is_multimodal(&self) -> bool {
        self.content_parts.is_some()
    }
}

/// Custom deserializer for Message to support OpenAI-style multimodal content.
///
/// This handles the case where `content` can be either:
/// - A string (text-only messages)
/// - An array of content parts (multimodal messages with images)
impl<'de> Deserialize<'de> for Message {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MessageVisitor;

        impl<'de> Visitor<'de> for MessageVisitor {
            type Value = Message;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a message object with role and content")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Message, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut role: Option<Role> = None;
                let mut content_str: Option<String> = None;
                let mut content_parts: Option<MessageContent> = None;
                let mut tool_calls: Option<Vec<ToolCall>> = None;
                let mut tool_call_id: Option<String> = None;
                let mut name: Option<String> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "role" => {
                            role = Some(map.next_value()?);
                        }
                        "content" => {
                            // Try to deserialize as string first, then as array
                            let value: serde_json::Value = map.next_value()?;
                            match value {
                                serde_json::Value::String(s) => {
                                    content_str = Some(s);
                                }
                                serde_json::Value::Array(arr) => {
                                    // Parse as content parts (multimodal)
                                    let parts: Vec<ContentPart> =
                                        serde_json::from_value(serde_json::Value::Array(arr))
                                            .map_err(de::Error::custom)?;
                                    content_parts = Some(MessageContent::Parts(parts));
                                }
                                serde_json::Value::Null => {
                                    content_str = Some(String::new());
                                }
                                _ => {
                                    return Err(de::Error::custom(
                                        "content must be a string or array of content parts",
                                    ))
                                }
                            }
                        }
                        "content_parts" => {
                            // Also support explicit content_parts field
                            content_parts = map.next_value()?;
                        }
                        "tool_calls" => {
                            tool_calls = map.next_value()?;
                        }
                        "tool_call_id" => {
                            tool_call_id = map.next_value()?;
                        }
                        "name" => {
                            name = map.next_value()?;
                        }
                        _ => {
                            // Ignore unknown fields
                            let _ = map.next_value::<serde_json::Value>();
                        }
                    }
                }

                // Extract text from content_parts if content string is not set
                let content = content_str.unwrap_or_else(|| {
                    content_parts
                        .as_ref()
                        .map(|cp| cp.extract_text())
                        .unwrap_or_default()
                });

                Ok(Message {
                    role: role.unwrap_or_default(),
                    content,
                    content_parts,
                    tool_calls,
                    tool_call_id,
                    name,
                })
            }
        }

        deserializer.deserialize_map(MessageVisitor)
    }
}

/// The role of a message sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// A message from the user
    #[default]
    User,
    /// A message from the AI assistant
    Assistant,
    /// A system message
    System,
    /// A tool response message
    Tool,
}
