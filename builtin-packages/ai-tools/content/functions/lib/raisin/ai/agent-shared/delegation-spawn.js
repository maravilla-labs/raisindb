/**
 * Spawning a child run through core (`raisin.agentRuns.spawnChild`).
 *
 * ai-tools decides WHAT the child is — the agent's delegation policy (depth,
 * fan-out, bounded parallelism, allowed agents), the narrowed tool and write
 * grant, the brief, the selected context — and writes the child's
 * conversation (its transcript). Core admits it on the parent (one commit:
 * the link, the budget reservation, the stored spawn plan), creates it, and
 * from then on owns the lineage: hand-back into the parent's mailbox, the
 * answer to a waiting tool, the cascade stop, crash repair.
 *
 * Idempotent: the spawn key is the model's `key` (or the task binding, or
 * the operation's digest), so a re-dispatched spawn gets the child it already
 * created back from core, and the conversation name derives from it too.
 */

import {
  normalizeSpawn, policyOf, agentPath, agentSlug, narrowTools, childChatName, spawnKeyOf,
  briefText, childDone, invalid, coreObjective, LIMITS,
} from './delegation-spec.js';
import { actingUser, runAgent } from './run-caller.js';
import { AI_WS, requireParentRun, ensureFolders, listChildren, snapshots } from './delegation-core.js';
import { resolveRunTools } from './run-tools.js';
import { DEFAULT_RUN_REDUCER, DEFAULT_MODEL_TURN_FUNCTION, PROJECT_FUNCTION } from './run-names.js';

function fail(message, cls) {
  return Object.assign(new Error(message), { error_class: cls });
}

async function loadAgent(path) {
  const agent = await raisin.nodes.get('functions', path);
  if (!agent || agent.node_type !== 'raisin:AIAgent') throw fail(`not an installed agent: ${path}`, 'not_found');
  return agent;
}

/** The last `n` user/assistant messages of a conversation, text only, bounded. */
async function recentTurns(workspace, chatPath, n) {
  const rows = await raisin.sql.query(`
    SELECT path, properties, created_at FROM '${workspace}'
    WHERE CHILD_OF($1) AND node_type = 'raisin:Message'
    ORDER BY created_at DESC LIMIT ${Math.min(Math.max(Number(n) || 6, 1), LIMITS.recent_turns) * 2}
  `, [chatPath]);
  const turns = [];
  for (const r of Array.isArray(rows) ? rows : []) {
    const p = r.properties || {};
    if (p.role !== 'user' && p.role !== 'assistant') continue;
    const body = typeof p.content === 'string' ? p.content
      : typeof p.body === 'string' ? p.body : (p.body && (p.body.content || p.body.message_text)) || '';
    const text = String(body || '').trim();
    if (!text) continue;
    turns.push({ role: p.role, text: text.slice(0, LIMITS.recent_chars) });
    if (turns.length >= n) break;
  }
  return turns.reverse();
}

/**
 * The selected context: core's `ContextSelection` (what the child record
 * carries) and the text of it the child's brief shows its model.
 */
async function selectContext(spec, parent) {
  if (spec.context_mode === 'recent' && parent.chatPath) {
    const turns = await recentTurns(parent.workspace, parent.chatPath, spec.recent_turns);
    const lines = turns.length
      ? ['Recent conversation of the delegating agent (most recent last):', ...turns.map((t) => `[${t.role}] ${t.text}`)]
      : [];
    if (spec.context !== null) lines.push(`Context provided by the delegating agent:\n${JSON.stringify(spec.context, null, 2)}`);
    return { selection: { mode: 'recent_turns', turns: spec.recent_turns, items: turns }, text: lines.join('\n') || null };
  }
  if (spec.context_mode === 'snapshot') {
    // Core writes a checkpoint of the parent and hands the child a reference
    // to it; the plan and the given facts go in beside it.
    const data = { plan: parent.view.projection || null, provided: spec.context };
    return {
      selection: { mode: 'snapshot', data },
      text: `Structured snapshot from the delegating run:\n${JSON.stringify(data, null, 2)}`,
    };
  }
  const text = spec.context !== null ? `Context provided by the delegating agent:\n${JSON.stringify(spec.context, null, 2)}` : null;
  return { selection: { mode: 'none' }, text };
}

/** Admission by the agent's delegation policy (core enforces its own limits too). */
function admit(spec, policy, depth, children) {
  if (depth >= policy.max_depth) {
    throw fail(`This run may not delegate${depth ? ' further' : ''}: delegation depth ${depth} reached the limit ${policy.max_depth}.`, 'unsupported');
  }
  if (children.length >= policy.max_children) {
    throw fail(`This run already started ${children.length} child run(s), the limit is ${policy.max_children}. Finish with what they returned.`, 'conflict');
  }
  const live = children.filter((c) => !childDone(c));
  if (live.length >= policy.max_parallel) {
    throw fail(`${live.length} child run(s) are still working (limit ${policy.max_parallel} at once). Wait for them with wait_for_agents first.`, 'conflict');
  }
  if (live.length && !spec.independent) {
    throw invalid(`A child run is still working (${live.map((c) => c.key || c.run_id).join(', ')}). Start another in parallel ONLY for work that does not depend on its result, and then set independent: true; otherwise wait for it first.`);
  }
  return live;
}

/** The child's conversation (its transcript) and its first message, the brief. */
async function writeTranscript(chatsRoot, chatName, props, brief, parentRunId) {
  const chatPath = `${chatsRoot}/${chatName}`;
  if (!(await raisin.nodes.get(AI_WS, chatPath))) {
    await ensureFolders(AI_WS, chatsRoot);
    try {
      await raisin.nodes.create(AI_WS, chatsRoot, { name: chatName, node_type: 'raisin:Conversation', properties: props });
    } catch (err) {
      if (!/already exists/i.test(String(err && err.message))) throw err;
    }
  }
  const briefPath = `${chatPath}/brief`;
  if (!(await raisin.nodes.get(AI_WS, briefPath))) {
    await raisin.nodes.create(AI_WS, chatPath, {
      name: 'brief',
      node_type: 'raisin:Message',
      properties: {
        role: 'user', content: brief, body: { content: brief, message_text: brief },
        message_type: 'delegation_brief', sender_id: `run:${parentRunId}`, status: 'delivered',
      },
    });
  }
  return { chatPath, briefPath, chat: await raisin.nodes.get(AI_WS, chatPath) };
}

/**
 * Spawn one child run for the calling run. Returns
 * `{ child, replayed, live_children, chat_path }`.
 */
export async function spawnChild(input, { functionPath, taskBinding = null } = {}) {
  const parent = await requireParentRun(input, functionPath);
  const spec = normalizeSpawn(input);
  if (taskBinding) spec.task_id = spec.task_id || taskBinding;
  const key = spawnKeyOf(spec, parent.opId);

  const parentAgentLoc = runAgent(parent.rec);
  const parentAgent = parentAgentLoc.path ? await raisin.nodes.get(parentAgentLoc.workspace, parentAgentLoc.path) : null;
  const parentProps = (parentAgent && parentAgent.properties) || {};
  const policy = policyOf(parentProps);
  const depth = Number(parent.rec.depth) || 0;

  const views = await listChildren(parent);
  const existing = views.find((v) => v.link && v.link.spawn_key === key);
  const children = await snapshots(parent, views);
  if (!existing) admit(spec, policy, depth, children);

  const childPath = spec.agent || agentPath(parentAgentLoc.path);
  if (!childPath) throw invalid('agent_ref is required (the calling run names no agent to copy)');
  if (policy.allowed_agents && !policy.allowed_agents.includes(childPath)) {
    throw fail(`${childPath} is not an agent this agent may delegate to (allowed: ${policy.allowed_agents.join(', ') || 'none'}).`, 'permission_denied');
  }
  const childProps = (await loadAgent(childPath)).properties || {};
  const mayDelegate = depth + 1 < Math.min(policy.max_depth, policyOf(childProps).max_depth);
  const hasSkills = Array.isArray(childProps.skills) && childProps.skills.length > 0;
  const { tools: agentTools } = await resolveRunTools(childProps, { hasSkills });
  const { tools, refused } = narrowTools(agentTools, spec.tools, { mayDelegate });
  if (refused.length) {
    throw invalid(`${childPath} does not offer: ${refused.join(', ')}. Its tools: ${agentTools.map((t) => t.name).join(', ') || 'none'}.`);
  }

  const slug = agentSlug(childPath);
  const user = actingUser(parent.rec);
  const context = await selectContext(spec, parent);
  const brief = briefText(spec, { contextBlock: context.text, parentAgent: parentProps.title || parentAgentLoc.path });
  const chatsRoot = `/agents/${slug}/inbox/chats`;
  const { chatPath, briefPath, chat } = await writeTranscript(chatsRoot, childChatName(parent.runId, key), {
    title: spec.objective.goal.slice(0, 120),
    agent_ref: childPath,
    // raisin:Conversation requires its participants. Only the agent: a
    // non-agent participant is read as the HUMAN a reply is delivered to.
    participants: [`agent:${slug}`],
    conversation_kind: 'delegation',
    parent_run_id: parent.runId,
    ...(user ? { delegated_for_user: user } : {}),
  }, brief, parent.runId);

  // Core narrows every call the child makes to its grant; the projection is
  // the reducer's own operation and is always allowed.
  const allowedTools = [...tools.filter((t) => t.kind !== 'domain' && t.function_path).map((t) => t.function_path), PROJECT_FUNCTION];
  const { on_exceeded: onExceeded, ...budgets } = spec.budget;
  const runConfig = childProps.run_config && typeof childProps.run_config === 'object' ? childProps.run_config : {};
  const spawned = await raisin.agentRuns.spawnChild({
    run_id: parent.runId,
    objective: coreObjective(spec, { brief, allowedTools, contextSelection: context.selection }),
    budgets,
    on_exceeded: onExceeded || 'fail',
    spawn_key: key,
    subject: { workspace: AI_WS, path: chatPath, ...(chat && chat.id ? { node_id: chat.id } : {}) },
    as_agent: `functions:${childPath}`,
    agent_ref: `functions:${childPath}`,
    reducer: { function_path: childProps.run_reducer || DEFAULT_RUN_REDUCER },
    executor_config: {
      model_turn_function: childProps.model_turn_function || DEFAULT_MODEL_TURN_FUNCTION,
      workspace: AI_WS,
      chat_path: chatPath,
    },
    // What the child's reducer starts from (core nests it under `input`).
    input: {
      text: brief,
      message_path: briefPath,
      tools,
      plan: null,
      config: {
        ...runConfig,
        execution_mode: 'automatic',
        requires_approval: false,
        ...(Array.isArray(spec.writes) ? { write_scope: spec.writes } : {}),
      },
      context: {
        workspace: AI_WS,
        chat_path: chatPath,
        agent_name: slug,
        sender_id: user,
        delegation: { parent_run_id: parent.runId, key, depth: depth + 1 },
      },
    },
  });
  if (!spawned || typeof spawned.child_run_id !== 'string') throw fail('the server did not start a child run', 'unsupported');
  await raisin.nodes.updateProperty(AI_WS, chatPath, 'active_agent_run_id', spawned.child_run_id);

  return {
    child: {
      run_id: spawned.child_run_id,
      child_no: spawned.child_no,
      key,
      task_id: spec.task_id,
      agent_ref: childPath,
      chat_path: chatPath,
      objective: spec.objective.goal,
      tools: tools.map((t) => t.name),
      budgets: spawned.budgets,
    },
    replayed: !spawned.created,
    chat_path: chatPath,
    live_children: children.filter((c) => !childDone(c)).map((c) => c.key || c.run_id),
  };
}
