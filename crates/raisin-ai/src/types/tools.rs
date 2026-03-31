//! Tool and function calling types.
//!
//! Contains [`ToolDefinition`], [`ToolCall`], and related types for
//! agentic workflows with tool/function calling.

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};

/// A tool call made by the assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for this tool call
    pub id: String,

    /// The type of tool call (usually "function")
    #[serde(rename = "type")]
    pub call_type: String,

    /// The function details
    pub function: FunctionCall,

    /// Streaming index for delta merging (provider's tool call position).
    /// Present only in streaming deltas; omitted from non-streaming responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

/// Details of a function call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    /// The name of the function to call
    pub name: String,

    /// JSON string of arguments.
    ///
    /// Accepts either a JSON string or a JSON object/value during deserialization.
    /// If an object/array/value is provided (e.g. from Starlark's auto-parsed JSON),
    /// it is serialized back to a string.
    #[serde(deserialize_with = "deserialize_string_or_value")]
    pub arguments: String,
}

/// Accept either a JSON string or a JSON object/value for arguments.
/// If a map/array/value is provided, serialize it back to a string.
fn deserialize_string_or_value<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Null => Ok("{}".to_string()),
        other => Ok(other.to_string()),
    }
}

/// A tool definition for agentic workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The type of tool (usually "function")
    #[serde(rename = "type")]
    pub tool_type: String,

    /// The function definition
    pub function: FunctionDefinition,
}

impl ToolDefinition {
    /// Creates a new function tool definition.
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_ai::types::ToolDefinition;
    /// use serde_json::json;
    ///
    /// let tool = ToolDefinition::function(
    ///     "get_weather".to_string(),
    ///     "Get the current weather".to_string(),
    ///     json!({
    ///         "type": "object",
    ///         "properties": {
    ///             "location": {
    ///                 "type": "string",
    ///                 "description": "The city name"
    ///             }
    ///         },
    ///         "required": ["location"]
    ///     })
    /// );
    /// ```
    pub fn function(name: String, description: String, parameters: serde_json::Value) -> Self {
        Self {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name,
                description,
                parameters,
            },
        }
    }

    /// Generates a tool guidance prompt from tool definitions.
    ///
    /// This creates a system prompt section that describes available tools
    /// and instructs the model on when to use them. The guidance is generated
    /// from the existing tool descriptions, making it maintainable.
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_ai::types::ToolDefinition;
    /// use serde_json::json;
    ///
    /// let tools = vec![
    ///     ToolDefinition::function(
    ///         "weather".to_string(),
    ///         "Get current weather for a location".to_string(),
    ///         json!({"type": "object"})
    ///     ),
    /// ];
    ///
    /// let guidance = ToolDefinition::generate_tool_guidance(&tools);
    /// assert!(guidance.contains("weather"));
    /// ```
    pub fn generate_tool_guidance(tools: &[ToolDefinition]) -> String {
        if tools.is_empty() {
            return String::new();
        }

        let tool_names: Vec<&str> = tools.iter().map(|t| t.function.name.as_str()).collect();

        let mut guidance = String::from("# CRITICAL: Tool Usage Rules\n\n");
        guidance.push_str("You have these tools available: ");
        guidance.push_str(&tool_names.join(", "));
        guidance.push_str("\n\n");

        guidance.push_str("## WHEN TO USE TOOLS:\n");
        guidance.push_str("- ONLY when the user explicitly asks for something a tool provides\n");
        guidance.push_str("- Example: \"weather\" tool → only for weather questions\n\n");

        guidance.push_str("## WHEN NOT TO USE TOOLS (respond directly instead):\n");
        guidance.push_str("- General knowledge questions (capitals, translations, facts, math)\n");
        guidance.push_str("- Questions about yourself or your capabilities\n");
        guidance.push_str("- Anything you can answer from your training data\n\n");

        guidance.push_str("## Tool Descriptions:\n");
        for tool in tools {
            guidance.push_str(&format!(
                "**{}**: {}\n",
                tool.function.name, tool.function.description
            ));
        }

        guidance
            .push_str("\n**IMPORTANT**: If unsure, DO NOT use a tool. Answer directly instead.\n");

        guidance
    }
}

/// A function definition for tool use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    /// The name of the function
    pub name: String,

    /// Description of what the function does
    pub description: String,

    /// JSON schema for the function parameters
    pub parameters: serde_json::Value,
}
