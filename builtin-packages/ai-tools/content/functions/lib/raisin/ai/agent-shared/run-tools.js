/**
 * Tool exposure for AgentRuns.
 *
 * Two halves of one contract:
 * - `resolveRunTools` (at run create): the agent's tools as the reducer's
 *   compact tool list — name, function path, kind, mutating, replay_safe. The
 *   planning tools become DOMAIN tools: under a run the reducer answers them
 *   and the plan is run state.
 * - `offerDefinitions` (in every model turn): a reducer's `tools_offered` as
 *   provider tool definitions. A function tool offered with a bare schema gets
 *   its real schema from its function node, so a reducer never has to carry
 *   schemas in its state.
 *
 * Invocation is core's: the reducer emits `call_tool` with the function path
 * and core executes it through the generic function executor, as the run's
 * principal.
 */

import { resolveToolsParallel } from './tools.js';
import { LOAD_SKILL_TOOL_NAME, LOAD_SKILL_FUNCTION } from './skills.js';
import { planOpOf } from './run-names.js';
import { REPLAY_SAFE_TOOL_PATHS } from './tool-meta.js';

const DESCRIPTION_CHARS = 1000;

/** One resolved tool ref → the reducer's tool entry. */
export function runToolOf(name, ref) {
  const path = ref['raisin:path'] || null;
  const op = planOpOf(path) || (ref.category === 'planning' ? planOpOf(name) : null);
  if (op) {
    return { name, kind: 'domain', domain_op: op, function_path: path, mutating: false, replay_safe: true };
  }
  const mutating = ref.mutating !== false;
  return {
    name,
    kind: 'function',
    function_path: path,
    mutating,
    replay_safe: !mutating || ref.idempotent === true || REPLAY_SAFE_TOOL_PATHS.includes(path),
    ...(ref.description ? { description: String(ref.description).slice(0, DESCRIPTION_CHARS) } : {}),
  };
}

/**
 * The agent's tools for a run: planning tools only with
 * `task_creation_enabled`, `load-skill` added when the agent
 * has skills and does not list it itself.
 */
export async function resolveRunTools(agentProps, { hasSkills = false } = {}) {
  const { toolNameToRef } = await resolveToolsParallel(agentProps.tools || []);
  const taskCreation = agentProps.task_creation_enabled === true;
  const tools = [];
  for (const [name, ref] of Object.entries(toolNameToRef)) {
    const entry = runToolOf(name, ref);
    if (entry.kind === 'domain' && !taskCreation) continue;
    tools.push(entry);
  }
  if (hasSkills && !tools.some((t) => t.name === LOAD_SKILL_TOOL_NAME)) {
    const extra = await resolveToolsParallel([
      { 'raisin:path': LOAD_SKILL_FUNCTION.path, 'raisin:workspace': LOAD_SKILL_FUNCTION.workspace },
    ]);
    for (const [name, ref] of Object.entries(extra.toolNameToRef)) tools.push(runToolOf(name, ref));
  }
  return { tools, hasPlanningTools: tools.some((t) => t.kind === 'domain') };
}

function trivialSchema(schema) {
  if (!schema || typeof schema !== 'object') return true;
  const props = schema.properties;
  return !props || typeof props !== 'object' || Object.keys(props).length === 0;
}

function definition(name, description, parameters) {
  const params = parameters && typeof parameters === 'object' ? { ...parameters } : { type: 'object', properties: {} };
  if (!params.type) params.type = 'object';
  if (!params.properties) params.properties = {};
  return { type: 'function', function: { name, description: description || '', parameters: params } };
}

/** `tools_offered` → provider definitions, in offer order. */
export async function offerDefinitions(toolsOffered) {
  const offers = Array.isArray(toolsOffered) ? toolsOffered.filter((t) => t && t.name) : [];
  const toResolve = offers.filter((t) => t.kind !== 'domain' && t.function_path && trivialSchema(t.schema));
  const byPath = new Map();
  if (toResolve.length) {
    const refs = toResolve.map((t) => ({ 'raisin:path': t.function_path, 'raisin:workspace': 'functions' }));
    const { toolDefinitions, toolNameToRef } = await resolveToolsParallel(refs);
    for (const def of toolDefinitions) {
      const ref = toolNameToRef[def.function.name];
      if (ref) byPath.set(ref['raisin:path'], def.function);
    }
  }
  return offers.map((t) => {
    const resolved = byPath.get(t.function_path);
    if (resolved) return definition(t.name, t.description || resolved.description, resolved.parameters);
    return definition(t.name, t.description, t.schema);
  });
}

/**
 * Whether `functionPath` is allowed by a run's `allowed_tools` (core's rule:
 * an exact path, or a prefix ending in `*`; an empty or absent list allows
 * everything).
 */
export function toolAllowed(allowed, functionPath) {
  if (!Array.isArray(allowed) || allowed.length === 0) return true;
  const path = String(functionPath || '');
  return allowed.some((a) => (typeof a === 'string' && a.endsWith('*') ? path.startsWith(a.slice(0, -1)) : a === path));
}

/**
 * The offers a run may make, under its `executor_config.allowed_tools` (a
 * child's grant from its parent). Domain tools are answered by the reducer and
 * never reach a function, so they stay; a function tool outside the grant is
 * not offered at all — core would refuse the call anyway.
 */
export function allowedOffers(toolsOffered, allowed) {
  const offers = Array.isArray(toolsOffered) ? toolsOffered : [];
  if (!Array.isArray(allowed) || allowed.length === 0) return offers;
  return offers.filter((t) => t && (t.kind === 'domain' || toolAllowed(allowed, t.function_path)));
}
