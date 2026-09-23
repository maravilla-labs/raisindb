/**
 * Per-tool envelope metadata for the ai-tools tools: whether a tool writes,
 * which nodes it writes, and what a model should do next.
 *
 * `REPLAY_SAFE_TOOL_PATHS` lists the tools that honour the operation id as an
 * idempotency key (or are idempotent by construction), so a run may
 * re-dispatch them after a worker dies mid-call. A tool NOT listed here is
 * dispatched with `replay_safe: false`: core abandons it on takeover and the
 * reducer re-reads instead of repeating a write.
 */

import { writeOf, locator } from './tool-envelope.js';

const AI = 'ai';
const ctxOf = (input) => (input && input.__raisin_context) || {};

const readOnly = (tool, next) => ({ tool, mutating: false, next });

export const TOOL_META = Object.freeze({
  weather: readOnly('weather'),
  searchDocuments: readOnly('search-documents', (r) => (
    r && Array.isArray(r.results) && r.results.length === 0
      ? [{ action: 'rephrase_query', reason: 'no passages matched' }]
      : []
  )),
  ask: readOnly('ask'),
  graphContext: readOnly('graph-context'),
  loadSkill: readOnly('load-skill'),
  getPlanStatus: readOnly('get-plan-status'),
  extractEntities: {
    tool: 'extract-entities',
    mutating: true,
    writes: (r) => (Array.isArray(r && r.entities)
      ? r.entities.filter((e) => e && e.path).slice(0, 50).map((e) => ({
        locator: locator(e.workspace || 'default', e.path, e.id || null), action: 'created',
      }))
      : []),
  },
  createPlan: {
    tool: 'create-plan',
    mutating: true,
    writes: (r, input) => writeOf(ctxOf(input).workspace || AI, r && r.plan_path, 'created', r && r.plan_id),
    next: (r) => (r && r.requires_approval ? [{ action: 'wait_for_approval', reason: 'the plan needs the user\'s approval' }] : []),
  },
  addTask: {
    tool: 'add-task',
    mutating: true,
    writes: () => [],
  },
  updateTask: {
    tool: 'update-task',
    mutating: true,
    writes: () => [],
  },
});

/** Function paths safe to re-dispatch under the same operation id. */
export const REPLAY_SAFE_TOOL_PATHS = Object.freeze([
  '/lib/raisin/ai/remember',
  '/lib/raisin/ai/forget',
  '/lib/raisin/ai/read-user-context',
  '/lib/raisin/ai/weather',
  '/lib/raisin/ai/search-documents',
  '/lib/raisin/ai/ask',
  '/lib/raisin/ai/graph-context',
  '/lib/raisin/ai/load-skill',
  '/lib/raisin/ai/get-plan-status',
  '/lib/raisin/ai/get-delegation-status',
  '/lib/raisin/ai/extract-entities',
  '/lib/raisin/ai/delegate-task',
  '/lib/raisin/ai/create-plan',
  '/lib/raisin/ai/add-task',
  // Delegation: keyed by the operation id (spawn, wait registration, control ids).
  '/lib/raisin/ai/spawn-agent',
  '/lib/raisin/ai/inspect-agent',
  '/lib/raisin/ai/message-agent',
  '/lib/raisin/ai/wait-for-agents',
  '/lib/raisin/ai/interrupt-agent',
]);
