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
  // The Rust accumulator returns provider-native and model-tagged reasoning
  // as one string. Agent persistence uses a list because a response may carry
  // multiple reasoning blocks, so normalize that boundary once here.
  if (Array.isArray(response.thinking)) {
    response.thinking = response.thinking
      .filter((thought) => typeof thought === 'string' && thought.trim())
      .map((thought) => thought.trim());
  } else if (typeof response.thinking === 'string' && response.thinking.trim()) {
    response.thinking = [response.thinking.trim()];
  } else {
    response.thinking = [];
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
 * Argument keys only the RUNTIME may set, and a model that writes one is
 * forging it. `__raisin_flow` ties a function's work to a flow instance (an
 * arming step compares it with the approval it recorded) and `_skill_grant` is
 * the side-band skill grant — the Rust loop strips both, and so does this one.
 * `__raisin_context` is deliberately NOT here: the chat handlers spread the
 * model's copy and then overwrite every key they own (load-skill trusts only
 * chat_path from it), which is the existing contract.
 */
const MODEL_FORBIDDEN_ARG_KEYS = ['__raisin_flow', '_skill_grant'];

/** Model-written tool arguments with every runtime-only key removed. */
function stripRuntimeArgs(args) {
  if (!args || typeof args !== 'object' || Array.isArray(args)) return args;
  const out = { ...args };
  for (const key of MODEL_FORBIDDEN_ARG_KEYS) delete out[key];
  return out;
}

/**
 * THE OFFER — name → function ref for exactly the tool definitions that were
 * sent to the model on the call that produced this response. That is the only
 * table a model's tool call may be resolved through.
 *
 * `toolNameToRef` is everything the agent was GIVEN, which is wider than what a
 * given turn OFFERS: the loop guard withdraws a repeated tool, a forced-final
 * turn offers none, and an agent without tools makes a plain completion. A
 * lookup straight into `toolNameToRef` executed all of those anyway — and, being
 * a plain object, answered `constructor` or `toString` with a truthy prototype
 * member. A Map built from own properties has neither hole.
 */
function offeredToolRefs(offeredDefinitions, toolNameToRef) {
  const offered = new Map();
  if (!Array.isArray(offeredDefinitions) || !toolNameToRef) return offered;
  for (const def of offeredDefinitions) {
    const name = def?.function?.name;
    if (typeof name !== 'string' || !name) continue;
    if (!Object.prototype.hasOwnProperty.call(toolNameToRef, name)) continue;
    const ref = toolNameToRef[name];
    if (ref && typeof ref === 'object') offered.set(name, ref);
  }
  return offered;
}

/** The ref an OFFERED tool resolves to — `null` for anything else. */
function resolveOfferedTool(offered, name) {
  if (!(offered instanceof Map) || typeof name !== 'string') return null;
  return offered.get(name) || null;
}

/**
 * What a model reads back when it calls a tool it was not offered: that
 * nothing ran, and what it CAN call. Same wording as the Rust loop's
 * `unoffered_tool_error`, so one agent reads one message on every path.
 */
function unofferedToolError(name, offered) {
  const names = offered instanceof Map ? [...offered.keys()].sort() : [];
  const choices = names.length === 0
    ? 'No tools are offered in this step.'
    : `The tools offered to you are: ${names.join(', ')}.`;
  return `\`${name}\` is not a tool offered to you, so it was not run. ${choices}`;
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
  MODEL_FORBIDDEN_ARG_KEYS,
  stripRuntimeArgs,
  offeredToolRefs,
  resolveOfferedTool,
  unofferedToolError,
  normalizeCompletionResponse,
  getToolCallName,
  normalizeToolCalls,
  parseToolArguments,
  resolveToolsParallel,
};
