/**
 * flow-agent-run — a flow's AI agent step, as a durable AgentRun.
 *
 * The flow runtime calls this when an agent step starts; it never calls a
 * model itself. Here the step becomes what every conversation already is: a
 * conversation holding the step's prompt, and an AgentRun of the agent over it
 * — the agent's own tools, budgets, stop/steer, leases and resume. The run
 * carries a WAITER naming the flow instance (the runtime's own `__raisin_flow`
 * stamp, never the step definition's), so when it ends core resumes the step
 * with its result through the job queue.
 *
 * Idempotent by (instance, step, visit): a re-executed step finds its run.
 */

import { AI_WS, ensureFolders } from '../agent-shared/delegation-core.js';
import { resolveRunTools } from '../agent-shared/run-tools.js';
import { DEFAULT_RUN_BUDGETS } from '../agent-shared/run-entry.js';
import {
  DEFAULT_RUN_REDUCER, DEFAULT_MODEL_TURN_FUNCTION, safeName, splitAgentRef,
} from '../agent-shared/run-names.js';

function fail(message) {
  return Object.assign(new Error(message), { error_class: 'config' });
}

/** The JSON schema a `response_format` asks for (OpenAI or plain shapes). */
export function outputSchemaOf(rf) {
  if (!rf || typeof rf !== 'object') return null;
  if (rf.json_schema && typeof rf.json_schema.schema === 'object') return rf.json_schema.schema;
  if (rf.schema && typeof rf.schema === 'object') return rf.schema;
  if (rf.type === 'json_object') return { type: 'object' };
  return null;
}

/** Deterministic conversation name of one visit of one step. */
export function flowChatName(flow, visit) {
  const inst = safeName(flow.instance_id).slice(0, 24);
  const step = safeName(flow.step_id).slice(0, 40);
  return `flow-${inst}-${step}-${Number(visit) || 0}`;
}

async function ensureNode(workspace, parent, name, nodeType, properties) {
  const path = `${parent}/${name}`;
  const existing = await raisin.nodes.get(workspace, path);
  if (existing) return existing;
  try {
    await raisin.nodes.create(workspace, parent, { name, node_type: nodeType, properties });
  } catch (err) {
    if (!/already exists/i.test(String(err && err.message))) throw err;
  }
  return raisin.nodes.get(workspace, path);
}

export async function handler(input = {}) {
  const flow = input.__raisin_flow;
  if (!flow || typeof flow.instance_id !== 'string' || typeof flow.step_id !== 'string') {
    throw fail('flow-agent-run is called by a flow agent step');
  }
  const { workspace: agentWs, path: agentPath } = splitAgentRef(input.agent_ref, input.agent_workspace || 'functions');
  const agent = agentPath ? await raisin.nodes.get(agentWs, agentPath) : null;
  if (!agent || agent.node_type !== 'raisin:AIAgent') throw fail(`not an installed agent: ${agentWs}:${agentPath}`);
  const props = agent.properties || {};
  const prompt = typeof input.prompt === 'string' ? input.prompt : '';
  const stepSkills = Array.isArray(input.skills) ? input.skills : [];
  const hasSkills = (Array.isArray(props.skills) && props.skills.length > 0) || stepSkills.length > 0;
  const { tools } = await resolveRunTools(props, { hasSkills });

  const slug = agentPath.split('/').filter(Boolean).pop() || 'agent';
  const chatsRoot = `/agents/${slug}/inbox/chats`;
  const name = flowChatName(flow, input.visit);
  const chatPath = `${chatsRoot}/${name}`;
  await ensureFolders(AI_WS, chatsRoot);
  const chat = await ensureNode(AI_WS, chatsRoot, name, 'raisin:Conversation', {
    title: (prompt || `Flow step ${flow.step_id}`).slice(0, 120),
    agent_ref: agentPath,
    // raisin:Conversation requires its participants. Only the agent: a
    // non-agent participant is read as the HUMAN a reply is delivered to.
    participants: [`agent:${slug}`],
    conversation_kind: 'flow',
    flow_instance_id: flow.instance_id,
    flow_step_id: flow.step_id,
  });
  const briefPath = `${chatPath}/prompt`;
  await ensureNode(AI_WS, chatPath, 'prompt', 'raisin:Message', {
    role: 'user', content: prompt, body: { content: prompt, message_text: prompt },
    message_type: 'flow_prompt', sender_id: `flow:${flow.instance_id}`, status: 'delivered',
  });

  const outputSchema = outputSchemaOf(input.response_format);
  const runConfig = props.run_config && typeof props.run_config === 'object' ? props.run_config : {};
  const maxCalls = Number(input.max_model_calls) > 0 ? Number(input.max_model_calls) : undefined;
  const created = await raisin.agentRuns.create({
    subject: { workspace: AI_WS, path: chatPath, ...(chat && chat.id ? { node_id: chat.id } : {}) },
    input: {
      text: prompt,
      message_path: briefPath,
      tools,
      plan: null,
      config: {
        ...runConfig,
        execution_mode: 'automatic',
        requires_approval: false,
        ...(outputSchema ? { output_schema: outputSchema } : {}),
      },
      context: {
        workspace: AI_WS, chat_path: chatPath, agent_name: slug,
        flow: { instance_id: flow.instance_id, step_id: flow.step_id },
      },
    },
    reducer: { function_path: props.run_reducer || DEFAULT_RUN_REDUCER },
    agent_ref: `${agentWs}:${agentPath}`,
    as_agent: `${agentWs}:${agentPath}`,
    create_key: `flow:${flow.instance_id}:${flow.step_id}:${Number(input.visit) || 0}`,
    budgets: {
      ...DEFAULT_RUN_BUDGETS,
      ...(props.run_budgets && typeof props.run_budgets === 'object' ? props.run_budgets : {}),
      ...(maxCalls ? { max_model_calls: maxCalls } : {}),
      // A flow step has nobody to ask for "continue": exceeding a budget ends it.
      on_exceeded: 'fail',
    },
    executor_config: {
      model_turn_function: props.model_turn_function || DEFAULT_MODEL_TURN_FUNCTION,
      workspace: AI_WS,
      chat_path: chatPath,
      ...(stepSkills.length ? { extra_skills: stepSkills } : {}),
    },
    waiter: { kind: 'flow_instance', target: flow.instance_id, branch: '', data: { step_id: flow.step_id } },
  });
  if (!created || typeof created.run_id !== 'string') throw fail('the server did not create the run');
  if (chat && chat.properties && chat.properties.active_agent_run_id !== created.run_id) {
    // Best effort, and SYNCHRONOUS in the runtime: no `.catch` on its result.
    try { await raisin.nodes.updateProperty(AI_WS, chatPath, 'active_agent_run_id', created.run_id); } catch (_) { /* a hint only */ }
  }
  return { run_id: created.run_id, created: created.created, status: created.status, chat_path: chatPath };
}
