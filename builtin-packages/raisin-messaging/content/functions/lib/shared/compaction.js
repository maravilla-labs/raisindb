/**
 * Chat History Compaction Utilities
 *
 * Provides functions for compacting long conversation histories to reduce
 * token usage in AI agent conversations. Older messages are summarized
 * and recent messages are kept intact.
 *
 * The `raisin` global is injected at runtime -- do NOT import it.
 */

// Default prompt for summarization
const DEFAULT_COMPACTION_PROMPT = `Summarize the following conversation history concisely, preserving:
- Key facts and decisions made
- Important context the assistant needs
- User preferences mentioned
- Any commitments or promises made

Keep it under 500 words. Output only the summary, no preamble.`;

/**
 * Estimate tokens using ~4 chars per token heuristic.
 *
 * @param {string} text - The text to estimate tokens for
 * @returns {number} Estimated token count
 */
export function estimate_tokens(text) {
  if (!text) {
    return 0;
  }
  if (typeof text !== "string") {
    text = String(text);
  }
  return Math.floor(text.length / 4);
}

/**
 * Estimate total tokens in a message array.
 *
 * @param {Array} messages - List of message dicts with "role" and "content" keys
 * @returns {number} Total estimated token count
 */
export function estimate_messages_tokens(messages) {
  if (!messages) {
    return 0;
  }

  let total = 0;
  for (const msg of messages) {
    let content = msg.content ?? "";
    if (typeof content === "object" && content !== null) {
      content = JSON.stringify(content);
    }
    total += estimate_tokens(content);

    // Add overhead for role, tool calls, etc.
    total += 10;

    // Add overhead for tool calls if present
    const tool_calls = msg.tool_calls ?? [];
    if (tool_calls.length > 0) {
      for (const tc of tool_calls) {
        const func = tc.function ?? {};
        total += estimate_tokens(func.name ?? "");
        total += estimate_tokens(func.arguments ?? "");
        total += 20; // Overhead for tool call structure
      }
    }
  }

  return total;
}

/**
 * Find a safe split index that doesn't break tool call sequences.
 *
 * A tool call sequence consists of:
 * 1. An assistant message with "tool_calls" field
 * 2. One or more immediately following "role: tool" messages
 *
 * @param {Array} messages - List of message dicts in OpenAI format
 * @param {number} target_index - The desired split position
 * @returns {number} Adjusted split index that is safe
 */
export function find_safe_split_index(messages, target_index) {
  if (target_index <= 0) {
    return 0;
  }
  if (target_index >= messages.length) {
    return messages.length;
  }

  let idx = target_index;

  // Scan backwards past tool messages
  for (let i = 0; i < messages.length; i++) {
    if (idx > 0 && messages[idx - 1].role === "tool") {
      idx--;
    } else {
      break;
    }
  }

  // Check if message just before is an assistant with tool_calls
  if (idx > 0) {
    const prev_msg = messages[idx - 1];
    if (prev_msg.role === "assistant" && prev_msg.tool_calls) {
      idx--;
    }
  }

  return idx;
}

/**
 * Validate and clean tool call history for AI API compatibility.
 *
 * Ensures every role="tool" message has a matching tool_call_id in a
 * preceding assistant message's tool_calls. Removes orphaned tool messages.
 *
 * @param {Array} messages - List of message dicts in OpenAI format
 * @returns {Array} Cleaned list of messages with no orphaned tool results
 */
export function validate_tool_history(messages) {
  if (!messages) {
    return messages;
  }

  // Build dict of valid tool_call_ids from assistant messages
  const valid_tool_call_ids = {};
  for (const msg of messages) {
    if (msg.role === "assistant") {
      const tool_calls = msg.tool_calls ?? [];
      for (const tc of tool_calls) {
        const tc_id = tc.id ?? "";
        if (tc_id) {
          valid_tool_call_ids[tc_id] = true;
        }
      }
    }
  }

  // Filter out orphaned tool messages
  const result = [];
  let removed_count = 0;
  for (const msg of messages) {
    if (msg.role === "tool") {
      const tool_call_id = msg.tool_call_id ?? "";
      if (!tool_call_id || !(tool_call_id in valid_tool_call_ids)) {
        console.log("warn", `[validate_tool_history] Removing orphaned tool message with tool_call_id: ${tool_call_id}`);
        removed_count++;
        continue;
      }
    }
    result.push(msg);
  }

  if (removed_count > 0) {
    console.log("info", `[validate_tool_history] Removed ${removed_count} orphaned tool messages`);
  }

  return result;
}

/**
 * Compact older messages if token threshold exceeded.
 *
 * @param {Array} messages - List of message dicts in OpenAI format
 * @param {object} agent_props - Agent configuration properties
 * @param {string} workspace - The workspace name (for caching summary)
 * @param {string} conversation_path - Path to the conversation node (for caching)
 * @returns {Array} Compacted list of messages
 */
export async function compact_history(messages, agent_props, workspace, conversation_path) {
  // Check if compaction is enabled
  if (!(agent_props.compaction_enabled ?? true)) {
    return messages;
  }

  const threshold = agent_props.compaction_token_threshold ?? 8000;
  const keep_recent = agent_props.compaction_keep_recent ?? 10;

  // Check if compaction is needed
  const total_tokens = estimate_messages_tokens(messages);
  if (total_tokens < threshold) {
    return messages;
  }

  console.log("info", `[compaction] Token count ${total_tokens} exceeds threshold ${threshold}, compacting history`);

  // Split: system prompt | older messages | recent messages
  let system_msg = null;
  let working_messages = messages;

  if (messages && messages.length > 0 && messages[0].role === "system") {
    system_msg = messages[0];
    working_messages = messages.slice(1);
  }

  // Not enough messages to compact
  if (working_messages.length <= keep_recent) {
    return messages;
  }

  // Calculate target split point
  const target_split = working_messages.length - keep_recent;

  // Adjust split point to avoid breaking tool call sequences
  const safe_split = find_safe_split_index(working_messages, target_split);

  // If safe_split means we'd keep almost everything in recent, skip compaction
  if (safe_split <= 2) {
    console.log("info", "[compaction] Safe split point too early, skipping compaction");
    return messages;
  }

  const older_messages = working_messages.slice(0, safe_split);
  const recent_messages = working_messages.slice(safe_split);

  // Try to use cached summary if available and still valid
  let summary = await _get_cached_summary(workspace, conversation_path, older_messages.length);

  if (summary) {
    console.log("info", "[compaction] Using cached summary");
  } else {
    // Generate new summary
    summary = await _generate_summary(older_messages, agent_props);

    // Cache the summary
    if (summary) {
      await _cache_summary(workspace, conversation_path, summary, older_messages.length);
    }
  }

  if (!summary) {
    console.log("warn", "[compaction] Failed to generate summary, returning original history");
    return messages;
  }

  // Build final history
  const result = [];
  if (system_msg) {
    result.push(system_msg);
  }

  result.push({
    role: "system",
    content: `[Previous conversation summary]\n${summary}\n[End of summary]`,
  });

  for (const msg of recent_messages) {
    result.push(msg);
  }

  console.log("info", `[compaction] Compacted ${messages.length} messages to ${result.length} messages`);
  return result;
}

/**
 * Get cached summary if valid.
 * @private
 */
async function _get_cached_summary(workspace, conversation_path, older_count) {
  const conv_node = await raisin.nodes.get(workspace, conversation_path);
  if (!conv_node) {
    return null;
  }

  const props = conv_node.properties ?? {};
  const cached_summary = props.compacted_summary;
  const cached_until = props.compacted_until_index ?? 0;

  // Check if cache covers enough messages
  if (cached_summary && cached_until >= older_count - 5) {
    return cached_summary;
  }

  return null;
}

/**
 * Store the compacted summary in the conversation node.
 * @private
 */
async function _cache_summary(workspace, conversation_path, summary, until_index) {
  await raisin.nodes.update(workspace, conversation_path, {
    properties: {
      compacted_summary: summary,
      compacted_until_index: until_index,
      compacted_at: new Date().toISOString(),
    },
  });
  console.log("info", `[compaction] Cached summary covering ${until_index} messages`);
}

/**
 * Generate summary of older messages using AI completion.
 * @private
 */
async function _generate_summary(messages, agent_props) {
  const custom_prompt = agent_props.compaction_prompt;
  const prompt = custom_prompt || DEFAULT_COMPACTION_PROMPT;

  // Format messages for summarization
  const formatted = _format_messages_for_summary(messages);

  // Build model ID
  const model_id = _build_model_id(agent_props);

  console.log("info", `[compaction] Generating summary using model: ${model_id}`);

  const response = await raisin.ai.completion({
    model: model_id,
    messages: [
      { role: "system", content: prompt },
      { role: "user", content: formatted },
    ],
    max_tokens: 1000,
    temperature: 0.3,
  });

  if (response && response.content) {
    return response.content;
  }

  return null;
}

/**
 * Build provider:model string from agent properties.
 * @private
 */
function _build_model_id(agent_props) {
  const compaction_provider = agent_props.compaction_provider;
  const compaction_model = agent_props.compaction_model;

  if (compaction_provider && compaction_model) {
    return `${compaction_provider}:${compaction_model}`;
  }

  if (compaction_model) {
    if (compaction_model.includes(":")) {
      return compaction_model;
    }
    const provider = agent_props.provider ?? "openai";
    return `${provider}:${compaction_model}`;
  }

  const provider = agent_props.provider ?? "openai";
  const model = agent_props.model ?? "gpt-4";
  return `${provider}:${model}`;
}

/**
 * Format messages into a readable format for summarization.
 * @private
 */
function _format_messages_for_summary(messages) {
  const parts = [];

  for (const msg of messages) {
    const role = msg.role ?? "unknown";
    let content = msg.content ?? "";

    if (typeof content === "object" && content !== null) {
      content = content.message_text || content.content || JSON.stringify(content);
    }

    if (role === "user") {
      parts.push(`User: ${content}`);
    } else if (role === "assistant") {
      parts.push(`Assistant: ${content}`);

      // Include tool calls if present
      const tool_calls = msg.tool_calls ?? [];
      if (tool_calls.length > 0) {
        for (const tc of tool_calls) {
          const func = tc.function ?? {};
          const func_name = func.name ?? "unknown";
          parts.push(`  [Called tool: ${func_name}]`);
        }
      }
    } else if (role === "tool") {
      // Tool results
      const tool_name = msg.name ?? "tool";
      const content_preview = content.length > 200 ? content.substring(0, 200) + "..." : content;
      parts.push(`  [Tool ${tool_name} returned: ${content_preview}]`);
    }
  }

  return parts.join("\n");
}

/**
 * Incrementally update an existing summary with new messages.
 *
 * @param {Array} messages - New messages to incorporate
 * @param {object} agent_props - Agent configuration properties
 * @param {string} workspace - The workspace name
 * @param {string} conversation_path - Path to the conversation node
 * @param {string} existing_summary - The existing summary to update
 * @returns {string|null} Updated summary or null on failure
 */
export async function incremental_compact(messages, agent_props, workspace, conversation_path, existing_summary) {
  const custom_prompt = agent_props.compaction_prompt;
  const base_prompt = custom_prompt || DEFAULT_COMPACTION_PROMPT;

  const prompt = base_prompt + "\n\nYou are updating an existing summary. Merge the new information with the existing summary.";

  const formatted_new = _format_messages_for_summary(messages);
  const formatted = `[Previous summary]\n${existing_summary}\n\n[New messages to incorporate]\n${formatted_new}`;

  const model_id = _build_model_id(agent_props);

  const response = await raisin.ai.completion({
    model: model_id,
    messages: [
      { role: "system", content: prompt },
      { role: "user", content: formatted },
    ],
    max_tokens: 1000,
    temperature: 0.3,
  });

  if (response && response.content) {
    const summary = response.content;
    await _cache_summary(workspace, conversation_path, summary, messages.length);
    return summary;
  }

  return null;
}
