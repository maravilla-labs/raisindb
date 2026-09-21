/**
 * load-skill — return one granted raisin:Skill's instructions.
 *
 * The prompt carries only the skills INDEX (agent-shared/skills.js); this is the
 * one door to a body. It answers ONLY for a skill the caller was given, and the
 * grant is derived on the server — never read from the model's arguments:
 *
 *   CHAT  — __raisin_context.chat_path → the Conversation's agent_ref → the
 *           agent node → the same resolver the prompt used. The chat handlers
 *           SPREAD any model-supplied __raisin_context first and then write
 *           chat_path themselves, so in chat nothing but chat_path is trusted —
 *           a forged `skill_grant` beside it is ignored.
 *   FLOW  — there is no chat_path. The flow runtime REPLACES __raisin_context on
 *           a load-skill call with {skill_grant: [{name, workspace, path}]},
 *           resolved by its Rust port of the same rule, and that list is used.
 *           This file cannot tell that list from one a model typed, so the
 *           runtime is the guard on EVERY flow path (agent_step, chat step,
 *           ai_container): it strips __raisin_context and _skill_grant from
 *           whatever the model wrote before attaching its own. A new path that
 *           runs a model's tool call must do the same.
 *
 * Row-level security remains the security boundary: this reads as the caller.
 * The refusal is scope discipline — an agent loads what it was given.
 *
 * Answers:
 *   {success:true, name, description, body}
 *   {success:false, reason:'not_granted', name, available}
 *   {success:false, reason:'not_found'|'disabled', name, available}
 *   {success:false, reason:'no_grant'}
 *
 * Execution mode: inline
 */

import {
  SKILL_NODE_TYPE,
  loadSkillGrant,
  grantEntries,
  skillBodyText,
} from '../agent-shared/skills.js';

function agentRefTarget(ref) {
  if (!ref) return null;
  if (typeof ref === 'string') return { workspace: 'functions', path: ref };
  const path = ref['raisin:path'] || (typeof ref['raisin:ref'] === 'string' && ref['raisin:ref'].startsWith('/') ? ref['raisin:ref'] : null);
  if (!path) return null;
  return { workspace: ref['raisin:workspace'] || 'functions', path };
}

/* The chat grant: the conversation's agent, resolved exactly as the prompt was. */
async function chatGrant(ctx) {
  const chatWorkspace = typeof ctx.workspace === 'string' && ctx.workspace ? ctx.workspace : 'ai';
  const chat = await raisin.nodes.get(chatWorkspace, ctx.chat_path);
  const target = agentRefTarget(chat && chat.properties && chat.properties.agent_ref);
  if (!target) return null;
  const agent = await raisin.nodes.get(target.workspace, target.path);
  if (!agent) return null;
  const skills = await loadSkillGrant({
    agentProps: agent.properties || {},
    stepSkills: [],
    getNode: (ws, path) => raisin.nodes.get(ws, path),
    getNodeById: typeof raisin.nodes.getById === 'function' ? (ws, id) => raisin.nodes.getById(ws, id) : undefined,
    getChildren: (ws, path) => raisin.nodes.getChildren(ws, path),
    onReadError: ({ workspace, path, error }) => {
      console.warn(`[load-skill] could not read ${workspace}:${path}: ${error && error.message}`);
    },
  });
  return grantEntries(skills);
}

/* The flow grant, as the runtime stated it. Entries that are not well formed are dropped. */
function flowGrant(ctx) {
  return ctx.skill_grant
    .filter((g) => g && typeof g.name === 'string' && typeof g.path === 'string')
    .map((g) => ({ name: g.name, workspace: typeof g.workspace === 'string' && g.workspace ? g.workspace : 'functions', path: g.path }));
}

export async function handler(input) {
  const args = input || {};
  const ctx = (args.__raisin_context && typeof args.__raisin_context === 'object') ? args.__raisin_context : {};
  const name = typeof args.name === 'string' ? args.name.trim() : '';

  let grant = null;
  if (typeof ctx.chat_path === 'string' && ctx.chat_path) {
    grant = await chatGrant(ctx);
  } else if (Array.isArray(ctx.skill_grant)) {
    grant = flowGrant(ctx);
  }
  if (!grant) return { success: false, reason: 'no_grant' };

  const available = grant.map((g) => g.name);
  const entry = grant.find((g) => g.name === name);
  if (!entry) return { success: false, reason: 'not_granted', name, available };

  const node = await raisin.nodes.get(entry.workspace, entry.path);
  const props = (node && node.properties) || {};
  if (!node || node.node_type !== SKILL_NODE_TYPE || props.name !== entry.name) {
    return { success: false, reason: 'not_found', name, available };
  }
  if (props.enabled === false) return { success: false, reason: 'disabled', name, available };

  return {
    success: true,
    name: props.name,
    description: typeof props.description === 'string' ? props.description : '',
    body: skillBodyText(props.body),
  };
}
