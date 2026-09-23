/**
 * agent-run-model-turn — ONE model turn of an AgentRun (the harness's
 * implementation of core's model-turn seam).
 *
 * Input (core, `FunctionModelTurnExecutor`):
 *   { run_id, operation_id, attempt, agent_ref, subject, executor_config,
 *     request: { tools_offered, instructions, tool_results, output_schema, context } }
 *   `context.restate_instructions: true` also puts `instructions` last (see context.js).
 * Output:
 *   { message:{text}, tool_calls:[{call_id,name,args}], finish_reason,
 *     usage:{input_tokens,output_tokens} }
 *   or { error_class, message, retryable }
 *
 * What a turn does, in order:
 *  1. replay guard — a re-dispatched operation returns the turn it already
 *     persisted (same message, same output, no second model call);
 *  2. transcript — writes the answers to the previous turn's tool calls, and
 *     marks steers the runtime consumed;
 *  3. context — system prompt + the run's authoritative facts + transcript;
 *     compaction to a STRUCTURED checkpoint when over budget; output budget;
 *  4. provider request with streaming and bounded retries;
 *  5. persists the assistant message (the transcript projection) and returns.
 *
 * It never decides what happens next: the reducer does.
 */

import { log, setContext } from '../agent-shared/logger.js';
import { buildHistoryFromChat } from '../agent-shared/history.js';
import { maybeCompactConversation } from '../agent-shared/compaction.js';
import { resolveAgentOutboxContext } from '../agent-shared/outbox.js';
import { createCostRecord } from '../agent-shared/utils.js';
import { loadSkillGrant } from '../agent-shared/skills.js';
import { memoryOwnerOf } from '../agent-shared/memory.js';
import { splitAgentRef, turnMessageName } from '../agent-shared/run-names.js';
import { offerDefinitions, allowedOffers } from '../agent-shared/run-tools.js';
import {
  loadRunChat, findPreviousTurn, writeToolResults, writeAssistantTurn, syncSteers, queuedSteerPaths, compactionName,
} from '../agent-shared/run-transcript.js';
import { buildSystemPrompt, applyBudget, contextBudget, estimateTokens, restateInstructions } from './context.js';
import { runModelTurn } from './provider.js';

async function loadAgent(agentRef) {
  const { workspace, path } = splitAgentRef(agentRef);
  if (!path) throw Object.assign(new Error('the run names no agent'), { error_class: 'config' });
  const agent = await raisin.nodes.get(workspace, path);
  if (!agent) throw Object.assign(new Error(`agent not found: ${workspace}:${path}`), { error_class: 'config' });
  return agent;
}

async function loadSkills(agentProps) {
  return loadSkillGrant({
    agentProps,
    getNode: (ws, p) => raisin.nodes.get(ws, p),
    getNodeById: raisin.nodes.getById ? (ws, id) => raisin.nodes.getById(ws, id) : undefined,
    getChildren: raisin.nodes.getChildren ? (ws, p) => raisin.nodes.getChildren(ws, p) : undefined,
    onReadError: ({ path, error }) => log.warn('model-turn', 'Skill unreadable', { path, error: String(error && error.message) }),
  }).catch(() => []);
}

/** Only the run's own operation may run a turn (this function runs as system). */
async function verifyRun(ctx) {
  if (!raisin.agentRuns || typeof raisin.agentRuns.get !== 'function') return null;
  const view = await raisin.agentRuns.get({ run_id: ctx.runId });
  const rec = view && view.run;
  const subject = rec && rec.subject;
  if (!subject || subject.path !== ctx.chatPath) {
    throw Object.assign(new Error('the run is not about this conversation'), { error_class: 'config' });
  }
  const op = rec.state && rec.state.activity && rec.state.activity.op;
  if (op && op.op_id && op.op_id !== ctx.operationId) {
    throw Object.assign(new Error(`operation ${ctx.operationId} is not the run's active operation`), { error_class: 'config' });
  }
  return rec;
}

function checkpointOf(input, facts) {
  return {
    run_id: input.run_id,
    operation_id: input.operation_id,
    facts: facts || null,
    large_refs: [],
  };
}

async function buildContext(ctx, agentProps, modelId, request, systemPrompt, excluded) {
  const opts = { maxHistoryMessages: agentProps.max_history_messages, excludeMessagePaths: excluded };
  let history = await buildHistoryFromChat(ctx.workspace, ctx.chatPath, systemPrompt, null, null, opts);
  const budget = contextBudget(agentProps);
  if (estimateTokens(history) > budget * 0.8) {
    const facts = request.context && request.context.facts;
    const compacted = await maybeCompactConversation(ctx.workspace, ctx.chatPath, agentProps, modelId, {
      force: true,
      always: true,
      nodeName: compactionName(ctx.runId, ctx.operationId),
      checkpoint: checkpointOf({ run_id: ctx.runId, operation_id: ctx.operationId }, facts),
    });
    if (compacted) {
      history = await buildHistoryFromChat(ctx.workspace, ctx.chatPath, systemPrompt, null, null, opts);
    }
  }
  return applyBudget(history, {
    budgetTokens: budget,
    maxToolChars: Number(agentProps.max_tool_result_chars) > 0 ? Number(agentProps.max_tool_result_chars) : undefined,
  });
}

export async function handler(input = {}) {
  const request = input.request || {};
  try {
    if (!input.run_id || !input.operation_id) {
      throw Object.assign(new Error('agent-run-model-turn is called by the run runtime'), { error_class: 'config' });
    }
    const base = await loadRunChat(input.subject);
    const ctx = { ...base, runId: input.run_id, operationId: input.operation_id };
    const rec = await verifyRun(ctx);
    setContext({ chat: ctx.chatPath, channel: ctx.streamChannel });

    // 1. Replay guard.
    const msgName = turnMessageName(ctx.runId, ctx.operationId);
    const already = await raisin.nodes.get(ctx.workspace, `${ctx.chatPath}/${msgName}`);
    if (already && already.properties && already.properties.run_turn_output) {
      log.info('model-turn', 'Replayed operation returns its persisted turn', { op: ctx.operationId });
      return already.properties.run_turn_output;
    }

    const agent = await loadAgent(input.agent_ref);
    const agentProps = agent.properties || {};
    const modelId = agentProps.provider ? `${agentProps.provider}:${agentProps.model}` : agentProps.model;
    const outboxCtx = await resolveAgentOutboxContext(ctx.workspace, ctx.chatPath, ctx.chat);

    // 2. Transcript: the previous turn's answers, and consumed steers.
    const facts = request.context && request.context.facts;
    const prev = await findPreviousTurn(ctx.workspace, ctx.chatPath, ctx.runId, facts && facts.last_turn_op);
    await writeToolResults(ctx, prev, request.tool_results);
    await syncSteers(ctx);
    const excluded = await queuedSteerPaths(ctx);

    // 3. Context.
    // A delegated run offers only the tools its parent granted (core refuses
    // any other call; offering it would only waste the model's turn).
    const offered = allowedOffers(request.tools_offered, input.executor_config && input.executor_config.allowed_tools);
    const definitions = await offerDefinitions(offered);
    // A flow step may lend the agent extra skills for its run.
    const extra = input.executor_config && Array.isArray(input.executor_config.extra_skills)
      ? input.executor_config.extra_skills : [];
    const skills = await loadSkills(extra.length
      ? { ...agentProps, skills: [...(Array.isArray(agentProps.skills) ? agentProps.skills : []), ...extra] }
      : agentProps);
    const memoryOwner = rec ? memoryOwnerOf(rec) : null;
    const systemPrompt = await buildSystemPrompt({ agentProps, request, memoryOwner, skills, definitions });
    const context = await buildContext(ctx, agentProps, modelId, request, systemPrompt, excluded);

    // 4. Provider request.
    const t0 = log.time();
    const maxTokens = Number(agentProps.max_output_tokens) > 0 ? Number(agentProps.max_output_tokens) : undefined;
    const complete = (messages, { tools, stream }) => raisin.ai.completion({
      messages,
      model: modelId,
      temperature: agentProps.temperature,
      ...(maxTokens ? { max_tokens: maxTokens } : {}),
      tools,
      stream,
      conversation_path: ctx.chatPath,
      conversation_channel: ctx.streamChannel || undefined,
    });
    const turn = await runModelTurn({
      complete,
      messages: restateInstructions(context.messages, request),
      tools: definitions,
      operationId: ctx.operationId,
      outputSchema: request.output_schema || null,
      transientRetries: agentProps.transient_retries === undefined || agentProps.transient_retries === null
        ? 2 : Math.max(0, Number(agentProps.transient_retries) || 0),
    });

    const output = {
      message: turn.message,
      tool_calls: turn.tool_calls,
      finish_reason: turn.finish_reason,
      usage: turn.usage,
      model: turn.model,
      context: { tokens: context.tokens, dropped: context.dropped },
      ...(turn.retries.length ? { retries: turn.retries } : {}),
    };

    // 5. Transcript projection of this turn.
    const node = await writeAssistantTurn(ctx, msgName, output, {
      senderId: outboxCtx ? outboxCtx.agentUserId : 'ai-assistant',
      senderName: outboxCtx ? outboxCtx.agentDisplayName : 'AI Assistant',
    });
    if (node && node.path) {
      await createCostRecord(ctx.workspace, ctx.chatPath, node.path, {
        model: turn.model, usage: { input_tokens: turn.usage.input_tokens, output_tokens: turn.usage.output_tokens },
      }, agentProps.provider, log.since(t0));
      if (agentProps.thinking_enabled && turn.thinking.length) {
        for (let i = 0; i < turn.thinking.length; i++) {
          // Best effort, and SYNCHRONOUS in the runtime: no `.catch` on its result.
          try {
            await raisin.nodes.create(ctx.workspace, node.path, {
              name: `thought-${i}`, node_type: 'raisin:AIThought', properties: { content: turn.thinking[i], thought_type: 'reasoning' },
            });
          } catch (_) { /* a thought is decoration, never the turn */ }
        }
      }
    }
    return output;
  } catch (err) {
    const message = String((err && err.message) || err || 'model turn failed');
    log.error('model-turn', 'Model turn failed', { error: message, class: err && err.error_class });
    return {
      error_class: (err && err.error_class) || 'provider',
      message,
      retryable: err && typeof err.retryable === 'boolean' ? err.retryable : false,
    };
  }
}
