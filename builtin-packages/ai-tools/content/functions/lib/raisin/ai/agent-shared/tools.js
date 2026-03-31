/**
 * Tool resolution and normalization for agent handlers.
 *
 * Handles the full lifecycle of tool references:
 *   1. Resolve raisin:Function nodes from tool refs (parallel)
 *   2. Build OpenAI-compatible tool definitions
 *   3. Normalize heterogeneous tool call shapes from different providers
 *   4. Parse arguments safely
 */

import { log } from './logger.js';

/** Tools that must never be sent to the model as callable functions. */
const MODEL_TOOL_DENYLIST = new Set([
  'plan-approval-handler',
]);

/** Path prefixes excluded from model tool lists. */
const MODEL_TOOL_PATH_DENYLIST = [
  '/lib/raisin/ai/plan-approval-handler',
];

/**
 * Normalize a raw AI completion response into a predictable shape.
 * Handles provider differences (e.g. stop_reason vs finish_reason).
 */
function normalizeCompletionResponse(raw) {
  const response = (raw && typeof raw === 'object') ? { ...raw } : {};
  if (!response.finish_reason && response.stop_reason) {
    response.finish_reason = response.stop_reason;
  }
  if (!Array.isArray(response.tool_calls)) {
    response.tool_calls = [];
  }
  if (typeof response.content !== 'string') {
    response.content = response.content == null ? '' : String(response.content);
  }
  if (typeof response.model !== 'string') {
    response.model = response.model ? String(response.model) : undefined;
  }
  // Defense-in-depth: strip model control tokens that may leak through
  // (primary stripping happens in the Rust streaming layer)
  response.content = response.content
    .replace(/<\|(python_tag|eom_id|eot_id|start_header_id|end_header_id|begin_of_text|end_of_text)\|>/g, '');
  return response;
}

/**
 * Extract a tool name from a tool call object, handling multiple provider
 * schemas (OpenAI, Anthropic, generic).
 */
function getToolCallName(toolCall) {
  if (!toolCall || typeof toolCall !== 'object') return null;
  const name = toolCall?.function?.name
    || toolCall?.name
    || toolCall?.function_name
    || toolCall?.tool_name
    || toolCall?.toolName;
  if (typeof name === 'string' && name.trim()) {
    return name.trim().replace(/[<>]/g, '');
  }
  return null;
}

/**
 * Normalize an array of tool calls into a consistent shape.
 * Returns { normalized, malformed } — malformed entries are logged
 * but excluded from the main list.
 */
function normalizeToolCalls(toolCalls) {
  if (!Array.isArray(toolCalls)) {
    return { normalized: [], malformed: [] };
  }

  const normalized = [];
  const malformed = [];

  for (const tc of toolCalls) {
    if (!tc || typeof tc !== 'object') {
      malformed.push(tc);
      continue;
    }

    const name = getToolCallName(tc);
    if (!name) {
      malformed.push(tc);
      continue;
    }

    const rawArgs = tc?.function?.arguments ?? tc?.arguments ?? tc?.input ?? {};
    const normalizedFunction =
      tc.function && typeof tc.function === 'object'
        ? { ...tc.function, name, arguments: rawArgs }
        : { name, arguments: rawArgs };

    normalized.push({
      ...tc,
      function: normalizedFunction,
      name,
      id: tc.id || tc.tool_call_id || tc.call_id || null,
    });
  }

  return { normalized, malformed };
}

/**
 * Parse tool arguments from a tool call — handles both string (JSON)
 * and object forms.
 */
function parseToolArguments(toolCall) {
  const raw = toolCall?.function?.arguments ?? toolCall?.arguments;
  if (raw == null) return {};
  if (typeof raw === 'string') {
    try {
      return JSON.parse(raw) || {};
    } catch (e) {
      const toolName = getToolCallName(toolCall) || 'unknown-tool';
      throw new Error(`Invalid tool arguments JSON for ${toolName}: ${e.message}`);
    }
  }
  if (typeof raw === 'object') return raw;
  throw new Error('Tool arguments must be a JSON object');
}

/**
 * Resolve an array of tool references (paths or reference objects) into
 * OpenAI-compatible tool definitions.  All lookups run in parallel via
 * Promise.all().
 *
 * @returns {{ toolDefinitions: Array, toolNameToRef: Object }}
 */
async function resolveToolsParallel(toolRefs) {
  const toolDefinitions = [];
  const toolNameToRef = {};

  if (!toolRefs || toolRefs.length === 0) {
    return { toolDefinitions, toolNameToRef };
  }

  const fetchPromises = toolRefs.map(async (toolRef) => {
    try {
      const toolWorkspace = typeof toolRef === 'object'
        ? (toolRef.workspace || toolRef['raisin:workspace'] || 'functions')
        : 'functions';
      const toolPath = typeof toolRef === 'object'
        ? (toolRef.target || toolRef['raisin:path'])
        : toolRef;

      if (!toolPath) return null;

      const funcNode = await raisin.nodes.get(toolWorkspace, toolPath);
      if (!funcNode || funcNode.node_type !== 'raisin:Function') return null;

      return { funcNode, toolWorkspace, toolPath };
    } catch (e) {
      log.error('tools', 'Failed to resolve tool', { error: e.message });
      return null;
    }
  });

  const results = await Promise.all(fetchPromises);

  for (const result of results) {
    if (!result) continue;

    const { funcNode, toolWorkspace, toolPath } = result;
    const props = funcNode.properties || {};
    const toolName = funcNode.name ? String(funcNode.name).trim() : '';
    if (!toolName) continue;

    // Enforce denylist
    if (MODEL_TOOL_DENYLIST.has(toolName)) {
      log.debug('tools', 'Skipping denylisted tool', { name: toolName, path: toolPath });
      continue;
    }
    if (MODEL_TOOL_PATH_DENYLIST.some(prefix => String(toolPath || '').startsWith(prefix))) {
      log.debug('tools', 'Skipping denylisted tool path', { name: toolName, path: toolPath });
      continue;
    }

    // Normalize input_schema — may arrive as string, null, or object
    let schema = props.input_schema;
    if (typeof schema === 'string') {
      try { schema = JSON.parse(schema); } catch (_) { schema = null; }
    }
    if (!schema || typeof schema !== 'object') {
      log.warn('tools', 'Tool definition missing schema', { name: toolName });
      schema = { type: 'object', properties: {} };
    }
    if (!schema.type) schema.type = 'object';

    toolDefinitions.push({
      type: 'function',
      function: {
        name: toolName,
        description: props.description ? String(props.description) : '',
        parameters: schema,
      },
    });

    toolNameToRef[toolName] = {
      'raisin:ref': funcNode.id,
      'raisin:workspace': toolWorkspace,
      'raisin:path': toolPath,
      execution_mode: props.execution_mode || 'async',
      category: props.category || null,
    };

    log.debug('tools', 'Resolved tool', { name: toolName, path: toolPath, mode: props.execution_mode || 'async' });
  }

  log.info('tools', 'Tool resolution complete', { resolved: toolDefinitions.length, total_refs: toolRefs.length });
  return { toolDefinitions, toolNameToRef };
}

export {
  MODEL_TOOL_DENYLIST,
  MODEL_TOOL_PATH_DENYLIST,
  normalizeCompletionResponse,
  getToolCallName,
  normalizeToolCalls,
  parseToolArguments,
  resolveToolsParallel,
};
