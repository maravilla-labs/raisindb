/**
 * Agent Continue Handler
 *
 * Triggered when a raisin:AIToolResult aggregated_result node
 * is created.  Continues the agentic conversation loop by:
 *
 *   1. Collecting all completed tool results for the current assistant message
 *   2. Rebuilding conversation history with those results
 *   3. Calling AI completion for the next turn
 *   4. Creating a new assistant message (deterministic naming: continue-{N}-{base})
 *   5. Dispatching any new tool calls (always async)
 *   6. Emitting terminal SSE events when the turn is done
 *
 * Multi-tool coordination is handled by the Rust AIToolResultAggregation handler.
 * This JS handler fires strictly on aggregated_result nodes.
 *
 * Input:
 *   { event: { type, node_id, node_type, node_path }, workspace }
 */

import { log, setContext } from '../agent-shared/logger.js';
import { buildHistoryFromChat } from '../agent-shared/history.js';
import {
  resolveToolsParallel,
  normalizeToolCalls,
  parseToolArguments,
  getToolCallName,
  normalizeCompletionResponse,
} from '../agent-shared/tools.js';
import {
  resolveAgentOutboxContext,
  sendAgentOutboxMessage,
  buildPlanOutboxData,
  buildTaskUpdateOutboxData,
} from '../agent-shared/outbox.js';
import {
  resolveStreamChannel,
  emitConversationEvent,
  emitAssistantTurnError,
  resumeTerminalSideEffects,
  setTerminalMarker,
} from '../agent-shared/streaming.js';
import { loadUserMemory, formatMemoryForPrompt } from '../agent-shared/memory.js';
import {
  safeJson,
  getPlanningSystemPromptAddition,
  TERMINAL_FALLBACK_TEXT,
  getEffectiveExecutionMode,
  requiresPlanApproval,
  shouldAutoRunTasks,
  shouldPauseAfterTask,
  updateOrchestrationState,
} from '../agent-shared/utils.js';

const MAX_CONTINUATION_DEPTH = 20;
const TOOL_LOOP_THRESHOLD = 3;

// ─────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────

/** Strip continue-N- and reply-to- prefixes to get the root message name. */
function getBaseMsgName(name) {
  let base = name;
  const m = base.match(/^continue-(\d+)-(.+)$/);
  if (m) base = m[2];
  if (base.startsWith('continue-after-')) base = base.slice('continue-after-'.length);
  if (base.startsWith('reply-to-')) base = base.slice('reply-to-'.length);
  return base;
}

/** Extract the numeric continuation index from a message name, or 0. */
function getContinuationCount(name) {
  const m = name.match(/^continue-(\d+)-/);
  return m ? parseInt(m[1], 10) : 0;
}

/**
 * Scan aggregated tool results for plan-completion signals.
 * Returns info object when all tasks are done, or null.
 */
function extractCompletedPlanInfo(toolResults) {
  if (!Array.isArray(toolResults) || toolResults.length === 0) return null;

  for (const item of toolResults) {
    const fname = item?.function_name;
    const r = item?.result && typeof item.result === 'object' ? item.result : null;
    if (!r) continue;

    const normalizedName = (fname || '').replace(/-/g, '_');
    if (normalizedName === 'get_plan_status' || normalizedName === 'update_task') {
      const total = Number(r.total_tasks || 0);
      const completed = Number(r.completed_tasks || 0);
      const pending = Number(r.pending_tasks || 0);
      const done =
        r.status === 'completed' ||
        r.plan_status === 'completed' ||
        (total > 0 && pending === 0 && completed >= total);
      if (done) {
        return {
          title: r.title || 'the plan',
          message: r.message || null,
          total_tasks: total,
          completed_tasks: completed,
        };
      }
    }
  }
  return null;
}

function extractPlanProgressInfo(toolResults) {
  if (!Array.isArray(toolResults) || toolResults.length === 0) return null;
  for (let i = toolResults.length - 1; i >= 0; i--) {
    const item = toolResults[i];
    const fname = item?.function_name;
    const r = item?.result && typeof item.result === 'object' ? item.result : null;
    if (!r) continue;
    const normalizedName = (fname || '').replace(/-/g, '_');
    if (normalizedName === 'get_plan_status' || normalizedName === 'update_task') {
      return {
        status: r.status || r.plan_status || null,
        total_tasks: Number(r.total_tasks || 0),
        completed_tasks: Number(r.completed_tasks || 0),
        pending_tasks: Number(r.pending_tasks || 0),
        task_status: r.new_status || r.status || null,
        title: r.title || null,
      };
    }
  }
  return null;
}

async function createCostRecord(workspace, parentPath, response, provider, durationMs) {
  if (!workspace || !parentPath || !response) return;
  const usage = (response.usage && typeof response.usage === 'object') ? response.usage : {};
  const inputTokens = Number(usage.prompt_tokens ?? usage.input_tokens ?? 0) || 0;
  const outputTokens = Number(usage.completion_tokens ?? usage.output_tokens ?? 0) || 0;
  const totalTokens = Number(usage.total_tokens ?? (inputTokens + outputTokens)) || 0;
  const costUsd = Number(usage.cost_usd ?? response.cost_usd);
  const props = {
    model: response.model || 'unknown',
    provider: provider || 'unknown',
    input_tokens: inputTokens,
    output_tokens: outputTokens,
    total_tokens: totalTokens,
    duration_ms: Number(durationMs || 0),
    timestamp: new Date().toISOString(),
    ...(Number.isFinite(costUsd) ? { cost_usd: costUsd } : {}),
  };
  try {
    await raisin.nodes.create(workspace, parentPath, {
      name: 'cost-record',
      node_type: 'raisin:AICostRecord',
      properties: props,
    });
  } catch (e) {
    if (!String(e?.message || '').includes('already exists')) throw e;
  }
}

function hashReplaySource(input) {
  const text = String(input || '');
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash += (hash << 1) + (hash << 4) + (hash << 7) + (hash << 8) + (hash << 24);
  }
  return (hash >>> 0).toString(16);
}

async function markQueuedMessageState(workspace, messagePath, state, extra = {}) {
  if (!workspace || !messagePath || !state) return;
  try {
    await raisin.nodes.updateProperty(workspace, messagePath, 'orchestration_queue_state', state);
    await raisin.nodes.updateProperty(
      workspace,
      messagePath,
      `orchestration_queue_${state}_at`,
      new Date().toISOString(),
    );
    for (const [key, value] of Object.entries(extra || {})) {
      await raisin.nodes.updateProperty(workspace, messagePath, key, value);
    }
  } catch (err) {
    log.warn('continue', 'Failed to update queued message state', {
      message_path: messagePath,
      state,
      error: err?.message || String(err),
    });
  }
}

function extractMessageText(message) {
  const props = message?.properties || {};
  if (typeof props.content === 'string' && props.content.trim()) return props.content.trim();
  if (typeof props.body === 'string' && props.body.trim()) return props.body.trim();
  if (props.body && typeof props.body === 'object') {
    const fromBody = props.body.content || props.body.message_text || '';
    if (typeof fromBody === 'string' && fromBody.trim()) return fromBody.trim();
  }
  return '';
}

async function findOldestQueuedUserMessage(workspace, chatPath) {
  const rows = await raisin.sql.query(`
    SELECT path, properties, created_at
    FROM '${workspace}'
    WHERE CHILD_OF($1)
      AND node_type = 'raisin:Message'
      AND properties->>'role'::STRING = 'user'
      AND properties->>'orchestration_queue_state'::STRING = 'queued'
    ORDER BY created_at ASC
    LIMIT 50
  `, [chatPath]);

  if (!Array.isArray(rows) || rows.length === 0) return null;

  rows.sort((a, b) => {
    const aOrder = Number(a?.properties?.orchestration_queue_order || 0);
    const bOrder = Number(b?.properties?.orchestration_queue_order || 0);
    if (aOrder !== bOrder) return aOrder - bOrder;
    return String(a?.created_at || '').localeCompare(String(b?.created_at || ''));
  });

  const queuedPath = rows[0]?.path;
  if (!queuedPath) return null;
  return raisin.nodes.get(workspace, queuedPath);
}

async function drainQueuedUserIntent(workspace, chatPath, assistantMessagePath) {
  if (!workspace || !chatPath || !assistantMessagePath) return;

  const assistant = await raisin.nodes.get(workspace, assistantMessagePath);
  const replaySourcePath = typeof assistant?.properties?.queued_original_message_path === 'string'
    ? assistant.properties.queued_original_message_path
    : null;

  if (replaySourcePath) {
    await markQueuedMessageState(workspace, replaySourcePath, 'consumed', {
      queued_replay_message_path: null,
    });
  }

  const queued = await findOldestQueuedUserMessage(workspace, chatPath);
  if (!queued?.path) return;

  const queuedProps = queued.properties || {};
  const replayName = `msg-queued-replay-${hashReplaySource(queued.path)}`;
  const replayPath = `${chatPath}/${replayName}`;
  const replayContent = extractMessageText(queued);

  let created = false;
  try {
    await raisin.nodes.create(workspace, chatPath, {
      name: replayName,
      node_type: 'raisin:Message',
      properties: {
        role: 'user',
        message_type: queuedProps.message_type || 'chat',
        status: 'delivered',
        content: replayContent,
        body: {
          content: replayContent,
          message_text: replayContent,
        },
        sender_id: queuedProps.sender_id || 'system',
        sender_display_name: queuedProps.sender_display_name || 'System',
        created_at: new Date().toISOString(),
        is_system_generated: true,
        queued_original_message_path: queued.path,
      },
    });
    created = true;
  } catch (err) {
    if (!String(err?.message || '').includes('already exists')) throw err;
  }

  await markQueuedMessageState(workspace, queued.path, 'replaying', {
    queued_replay_message_path: replayPath,
  });

  log.info('continue', 'Queued replay message ready', {
    source_message: queued.path,
    replay_message: replayPath,
    created,
  });
}

/**
 * Send ai_plan / ai_task_update notifications from async tool results.
 * Idempotent per tool_call_id to prevent duplicates on trigger retries.
 */
async function emitToolResultsToOutbox(workspace, outboxCtx, assistantMsgPath, toolResults) {
  for (const tr of toolResults || []) {
    const toolName = tr?.function_name || '';
    const toolArgs = tr?.arguments || {};
    const result = tr?.result;
    const toolCallId = tr?.tool_call_id;
    if (!toolCallId || !result || tr?.error) continue;

    const planData = buildPlanOutboxData(toolName, toolArgs, result);
    if (planData) {
      await sendAgentOutboxMessage(
        workspace,
        outboxCtx,
        planData.title || 'Plan',
        'ai_plan',
        planData,
        { dedupe_key: `ai_plan:${assistantMsgPath}:${toolCallId}` },
      );
    }

    const taskUpdateData = buildTaskUpdateOutboxData(toolName, toolArgs, result);
    if (taskUpdateData) {
      await sendAgentOutboxMessage(
        workspace,
        outboxCtx,
        taskUpdateData.title || 'Task update',
        'ai_task_update',
        taskUpdateData,
        { dedupe_key: `ai_task_update:${assistantMsgPath}:${toolCallId}` },
      );
    }
  }
}

/**
 * Detect tool loops: if the most recent N assistant messages all issued
 * the exact same set of tool calls, the model is stuck.
 */
function detectToolLoop(history) {
  const rounds = [];
  for (let i = history.length - 1; i >= 0 && rounds.length < TOOL_LOOP_THRESHOLD + 1; i--) {
    const msg = history[i];
    if (msg.role === 'assistant' && msg.tool_calls && msg.tool_calls.length > 0) {
      rounds.unshift(msg.tool_calls.map(tc => tc.function?.name || '').sort().join(','));
    } else if (msg.role === 'tool') {
      continue; // skip tool result entries
    } else {
      break; // user or system message — stop scanning
    }
  }
  if (rounds.length >= TOOL_LOOP_THRESHOLD && rounds.every(r => r === rounds[0])) {
    return rounds[0];
  }
  return null;
}

// ─────────────────────────────────────────────────
// Tool call dispatch (shared between handler + continue-handler)
// ─────────────────────────────────────────────────

/**
 * Create an AIToolCall + immediate error result for an unknown or bad-args tool.
 * Returns true so the caller can track that a continuation is expected.
 */
async function createErrorToolResult(workspace, parentPath, toolCallName, toolCallId, toolName, errorObj) {
  await raisin.nodes.create(workspace, parentPath, {
    name: toolCallName,
    node_type: 'raisin:AIToolCall',
    properties: {
      tool_call_id: toolCallId,
      function_name: toolName,
      arguments: {},
      status: 'completed',
    },
  });
  await raisin.nodes.create(workspace, `${parentPath}/${toolCallName}`, {
    name: 'result',
    node_type: 'raisin:AIToolSingleCallResult',
    properties: {
      tool_call_id: toolCallId,
      function_name: toolName,
      result: errorObj,
      status: 'completed',
    },
  });
}

// ─────────────────────────────────────────────────
// Main handler
// ─────────────────────────────────────────────────

async function handleToolResult(ctx) {
  const { event, workspace } = ctx.flow_input;
  if (workspace !== 'ai') {
    log.debug('continue', 'Skipping continuation outside ai workspace', { workspace });
    return;
  }

  const resultPath = event.node_path;
  const pathParts = resultPath.split('/');
  const parentPath = pathParts.slice(0, -1).join('/');
  const nodeName = pathParts[pathParts.length - 1];

  if (nodeName !== 'aggregated_result') {
    log.debug('continue', 'Skipping non-aggregated tool result event', {
      result_path: resultPath,
      node_name: nodeName,
    });
    return;
  }

  const assistantMsgPath = parentPath;
  const chatPath = pathParts.slice(0, -2).join('/');

  if (!chatPath.startsWith('/agents/')) {
    log.debug('continue', 'Skipping non-agent conversation path', { chat: chatPath });
    return;
  }

  setContext({ chat: chatPath });
  log.info('continue', 'Triggered', { result_path: resultPath, aggregated: true });

  // ── Step 1: Load assistant message ──
  const assistantMsgName = assistantMsgPath.split('/').pop();
  log.step('continue', 1, 6, 'Loading assistant context', { continuation_depth: getContinuationCount(assistantMsgName) });

  let streamChannel = resolveStreamChannel(chatPath);
  // Safety-net state tracked across try/catch/finally blocks.
  // Must live in function scope (not block scope) so finally can read it.
  let pendingToolCount = 0;
  let continuationExpected = false;
  let isWaiting = false;

  const assistantMsg = await raisin.nodes.get(workspace, assistantMsgPath);
  if (!assistantMsg || assistantMsg.properties?.role !== 'assistant') {
    log.error('continue', 'Parent is not an assistant message', { path: assistantMsgPath });
    return;
  }

  await updateOrchestrationState(workspace, assistantMsgPath, {
    dispatch_phase: 'ready_for_model',
  });

  // Propagate plan_action_id through continuations
  const planActionId =
    typeof assistantMsg.properties?.plan_action_id === 'string' && assistantMsg.properties.plan_action_id
      ? assistantMsg.properties.plan_action_id
      : null;
  const queuedOriginalMessagePath =
    typeof assistantMsg.properties?.queued_original_message_path === 'string'
      ? assistantMsg.properties.queued_original_message_path
      : null;
  const isApprovalGateTurn = assistantMsg.properties.finish_reason === 'awaiting_plan_approval';

  // ── Step 2: Collect tool results ──
  let aggregatedToolResults = null;
  const aggNode = await raisin.nodes.get(workspace, resultPath);
  if (aggNode?.properties?.results) {
    aggregatedToolResults = aggNode.properties.results;
  }
  if (!Array.isArray(aggregatedToolResults)) {
    throw new Error(`aggregated_result node missing results array: ${resultPath}`);
  }
  log.step('continue', 2, 6, 'Collecting tool results', { result_count: aggregatedToolResults.length });

  // ── Continuation naming ──
  const baseName = getBaseMsgName(assistantMsgName);
  const prevCount = getContinuationCount(assistantMsgName);
  const nextCount = prevCount + 1;

  if (nextCount > MAX_CONTINUATION_DEPTH) {
    log.warn('continue', 'Max continuation depth reached', { depth: MAX_CONTINUATION_DEPTH });
    const depthMsg = `I've reached the maximum number of tool continuation steps (${MAX_CONTINUATION_DEPTH}). Please send a new message to continue.`;
    await raisin.nodes.create(workspace, chatPath, {
      name: `continue-${nextCount}-${baseName}`,
      node_type: 'raisin:Message',
      properties: {
        role: 'assistant',
        body: { content: depthMsg, message_text: depthMsg },
        content: depthMsg,
        sender_id: 'ai-assistant',
        sender_display_name: 'AI Assistant',
        message_type: 'chat',
        status: 'delivered',
        created_at: new Date().toISOString(),
        finish_reason: 'max_continuation_depth',
        ...(planActionId ? { plan_action_id: planActionId } : {}),
        parent_message_path: assistantMsgPath,
        continuation_depth: nextCount,
        error_details: { type: 'max_depth', depth: MAX_CONTINUATION_DEPTH },
      },
    });
    await emitConversationEvent('conversation:done', {
      type: 'done',
      content: depthMsg,
      role: 'assistant',
      senderDisplayName: 'AI Assistant',
      finishReason: 'max_continuation_depth',
      timestamp: new Date().toISOString(),
    }, chatPath, streamChannel);
    return;
  }

  const continuationMsgName = `continue-${nextCount}-${baseName}`;

  // Idempotency: if the continuation already exists, just resume terminal effects
  try {
    const existing = await raisin.nodes.get(workspace, `${chatPath}/${continuationMsgName}`);
    if (existing) {
      log.debug('continue', 'Skipping: continuation already exists', { name: continuationMsgName });
      const chatForExisting = await raisin.nodes.get(workspace, chatPath);
      const chan = resolveStreamChannel(chatPath, chatForExisting);
      const oCtx = await resolveAgentOutboxContext(workspace, chatPath, chatForExisting);
      await resumeTerminalSideEffects(workspace, chatPath, existing, oCtx, chan);
      if (existing.properties?.dispatch_phase === 'terminal'
          && existing.properties?.finish_reason !== 'awaiting_plan_approval'
          && existing.properties?.finish_reason !== 'awaiting_step_continue') {
        await drainQueuedUserIntent(workspace, chatPath, existing.path);
      }
      return;
    }
  } catch (_) { /* doesn't exist — proceed */ }

  // ── Load chat + agent config ──
  const chat = await raisin.nodes.get(workspace, chatPath);
  if (!chat || !chat.properties.agent_ref) {
    throw new Error(`Chat not found or missing agent_ref: ${chatPath}`);
  }
  streamChannel = resolveStreamChannel(chatPath, chat);
  setContext({ channel: streamChannel, chat: chatPath });

  let terminalEventEmitted = false;
  try {

  const agentRef = chat.properties.agent_ref;
  const agentPath = typeof agentRef === 'string' ? agentRef : agentRef['raisin:path'];
  const agentWorkspace = typeof agentRef === 'object' ? (agentRef['raisin:workspace'] || 'functions') : 'functions';
  const agent = await raisin.nodes.get(agentWorkspace, agentPath);
  if (!agent) throw new Error(`Agent not found: ${agentPath}`);
  const executionMode = getEffectiveExecutionMode(agent.properties.execution_mode);

  const outboxCtx = await resolveAgentOutboxContext(workspace, chatPath, chat);
  let { toolDefinitions, toolNameToRef } = await resolveToolsParallel(agent.properties.tools || []);

  // Filter planning tools when task_creation_enabled is off
  const taskCreationEnabled = agent.properties.task_creation_enabled === true;
  if (!taskCreationEnabled) {
    const planNames = Object.entries(toolNameToRef)
      .filter(([, ref]) => ref.category === 'planning')
      .map(([n]) => n);
    if (planNames.length > 0) {
      log.debug('continue', 'Filtering planning tools', { tools: planNames.join(', ') });
      toolDefinitions = toolDefinitions.filter(td => !planNames.includes(td.function?.name));
      for (const n of planNames) delete toolNameToRef[n];
    }
  }
  const hasPlanningTools = taskCreationEnabled && Object.values(toolNameToRef).some(r => r.category === 'planning');

  // Emit outbox notifications for completed tool results
  if (outboxCtx) {
    await emitToolResultsToOutbox(workspace, outboxCtx, assistantMsgPath, aggregatedToolResults);
  }

  // Emit tool_call_completed SSE for each async tool result
  for (const toolResult of aggregatedToolResults) {
    const tcId = toolResult.tool_call_id;
    const tcName = toolResult.function_name;
    if (!tcId) continue;
    await emitConversationEvent('conversation:tool_call_completed', {
      type: 'tool_call_completed',
      toolCallId: tcId,
      functionName: tcName,
      result: toolResult.result ?? null,
      error: toolResult.error ?? null,
      timestamp: new Date().toISOString(),
    }, chatPath, streamChannel);
  }

  // For plan-approval turns, project async results (ai_plan/updates) and stop.
  if (isApprovalGateTurn) {
    await updateOrchestrationState(workspace, assistantMsgPath, {
      dispatch_phase: 'terminal',
      terminal_reason_internal: 'awaiting_plan_approval',
    });
    const refreshed = await raisin.nodes.get(workspace, assistantMsgPath);
    await resumeTerminalSideEffects(workspace, chatPath, refreshed, outboxCtx, streamChannel);
    log.debug('continue', 'Plan approval gate turn processed; skipping model continuation');
    return;
  }

  // ── Step 3: Build conversation history ──
  const completedPlan = extractCompletedPlanInfo(aggregatedToolResults);
  const planProgress = extractPlanProgressInfo(aggregatedToolResults);
  const forceFinalText = !!completedPlan;

  let systemPrompt = agent.properties.system_prompt;
  if (hasPlanningTools && !forceFinalText) {
    systemPrompt = systemPrompt
      ? systemPrompt + '\n' + getPlanningSystemPromptAddition(executionMode)
      : getPlanningSystemPromptAddition(executionMode);
  }

  // Inject user memory
  if (outboxCtx) {
    const mem = await loadUserMemory(chatPath.split('/')[2], outboxCtx.senderId);
    if (mem) {
      const block = formatMemoryForPrompt(mem);
      systemPrompt = systemPrompt ? systemPrompt + block : block;
      log.debug('continue', 'Injected user memory into system prompt');
    }
  }

  // Inject agent rules
  const rules = agent.properties.rules;
  if (Array.isArray(rules) && rules.length > 0) {
    const rulesBlock = '\n\n## Rules\n' + rules.map(r => `- ${r}`).join('\n');
    systemPrompt = systemPrompt ? systemPrompt + rulesBlock : rulesBlock;
  }

  const t0Hist = log.time();
  let history = await buildHistoryFromChat(workspace, chatPath, systemPrompt, assistantMsgPath, aggregatedToolResults);

  if (forceFinalText) {
    const hint = completedPlan.message
      ? `Plan status indicates completion (${completedPlan.completed_tasks || 0}/${completedPlan.total_tasks || 0}). ${completedPlan.message} Provide a final user-facing summary now. Do not call tools.`
      : `Plan status indicates completion (${completedPlan.completed_tasks || 0}/${completedPlan.total_tasks || 0}). Provide a final user-facing summary now. Do not call tools.`;
    history = [...history, { role: 'system', content: hint }];
  }
  log.step('continue', 3, 6, 'Building conversation history', { message_count: history.length, duration_ms: log.since(t0Hist) });

  // Tool loop detection
  let toolLoopDetected = false;
  const loopedToolNames = detectToolLoop(history);
  if (loopedToolNames) {
    toolLoopDetected = true;
    log.warn('continue', `Tool loop detected: ${loopedToolNames} repeated ${TOOL_LOOP_THRESHOLD}+ times`);
  }

  // Build model ID
  const modelId = agent.properties.provider
    ? `${agent.properties.provider}:${agent.properties.model}`
    : agent.properties.model;

  const toolsEnabled = !toolLoopDetected && !forceFinalText && toolDefinitions.length > 0;
  if (!toolsEnabled) {
    history = [...history, { role: 'system', content: 'Tools are unavailable for this turn. Respond with plain text only. Do not call any function.' }];
  }

  // ── Step 4: AI completion ──
  log.step('continue', 4, 6, 'Calling AI completion', { model: modelId, tools_enabled: toolsEnabled, loop_detected: toolLoopDetected });
  const t0AI = log.time();
  let response;
  let completionError = null;

  try {
    const raw = await raisin.ai.completion({
      messages: history,
      model: modelId,
      temperature: agent.properties.temperature,
      tools: toolsEnabled ? toolDefinitions : undefined,
      stream: true,
      conversation_path: chatPath,
      conversation_channel: streamChannel || undefined,
    });
    response = normalizeCompletionResponse(raw);

    // Only inject fallback for forced-final summaries.
    if (forceFinalText && (!response.content || !response.content.trim()) && (!response.tool_calls || response.tool_calls.length === 0)) {
      response.content = `Completed ${completedPlan?.title || 'the plan'} (${completedPlan?.completed_tasks || 0}/${completedPlan?.total_tasks || 0} tasks).`;
      response.finish_reason = response.finish_reason || 'stop';
    }
  } catch (err) {
    completionError = err;
  }

  // Retry on failed_generation with function syntax when tools are disabled
  if (!response && completionError && !toolsEnabled) {
    const errMsg = String(completionError?.message || '');
    if (errMsg.includes('failed_generation') && errMsg.includes('<function=')) {
      try {
        const retryRaw = await raisin.ai.completion({
          messages: [...history, { role: 'system', content: 'Return a normal text response only. No function calls, no tool syntax, no XML tags.' }],
          model: modelId,
          temperature: agent.properties.temperature,
          tools: undefined,
          stream: true,
          conversation_path: chatPath,
          conversation_channel: streamChannel || undefined,
        });
        response = normalizeCompletionResponse(retryRaw);
      } catch (retryErr) {
        completionError = retryErr;
      }
    }
  }

  if (!response && completionError) {
    log.error('continue', 'AI completion failed', { error: completionError.message, duration_ms: log.since(t0AI) });
    await emitAssistantTurnError(workspace, chatPath, continuationMsgName, completionError.message || String(completionError), outboxCtx, streamChannel);
    terminalEventEmitted = true;
    throw completionError;
  }

  log.step('continue', 5, 6, 'AI response received', { finish_reason: response.finish_reason, tool_calls: response.tool_calls.length, duration_ms: log.since(t0AI) });

  // Normalize tool calls
  const { normalized, malformed } = normalizeToolCalls(response.tool_calls);
  response.tool_calls = normalized;
  if (malformed.length > 0) {
    log.warn('continue', 'Dropped malformed tool call entries', { count: malformed.length, entries: safeJson(malformed) });
    if (normalized.length === 0 && !response.content.trim()) {
      response.content = 'I received an invalid tool call payload from the model. Please try again.';
      response.finish_reason = response.finish_reason || 'stop';
    }
  }

  // Detect raw function syntax in content (model quirk, e.g. Llama)
  if (response.tool_calls.length === 0 && /<function=[\w-]+>/.test(response.content || '')) {
    log.warn('continue', 'Model emitted raw <function=...> syntax, retrying without tools');
    try {
      const retryRaw = await raisin.ai.completion({
        messages: [...history,
          { role: 'assistant', content: response.content },
          { role: 'user', content: 'Your previous response used invalid function call syntax. Respond in plain text.' },
        ],
        model: modelId,
        temperature: agent.properties.temperature,
        tools: undefined,
        stream: false,
        conversation_path: chatPath,
        conversation_channel: streamChannel || undefined,
      });
      const retryResp = normalizeCompletionResponse(retryRaw);
      if (retryResp.content?.trim() && !/<function=[\w-]+>/.test(retryResp.content)) {
        response.content = retryResp.content;
        log.info('continue', 'Retry after raw function syntax succeeded');
      }
    } catch (retryErr) {
      log.warn('continue', 'Retry after raw function syntax failed', { error: retryErr.message });
    }
    if (/<function=[\w-]+>/.test(response.content || '')) {
      response.content = (response.content || '').replace(/<function=[\w-]+>[\s\S]*?<\/function>/g, '').trim();
    }
  }

  // Detect tool-echo: model wrote "Calling update-task" as text instead of making the call.
  // Retry with tools enabled and a nudge to use proper function calling.
  if (toolsEnabled && response.tool_calls.length === 0 && /^Calling\s+[\w-]+/i.test((response.content || '').trim())) {
    log.warn('continue', 'Model echoed tool name as text instead of calling it, retrying with nudge', { content: response.content.trim() });
    try {
      const retryRaw = await raisin.ai.completion({
        messages: [...history,
          { role: 'assistant', content: response.content },
          { role: 'system', content: 'You wrote the tool name as plain text instead of actually calling it. You MUST use the function calling mechanism to invoke tools. Do NOT write "Calling ..." as text. Actually call the function now and continue executing the plan.' },
        ],
        model: modelId,
        temperature: agent.properties.temperature,
        tools: toolDefinitions,
        stream: false,
        conversation_path: chatPath,
        conversation_channel: streamChannel || undefined,
      });
      const retryResp = normalizeCompletionResponse(retryRaw);
      if (retryResp.tool_calls?.length > 0 || retryResp.content?.trim()) {
        const retryNorm = normalizeToolCalls(retryResp.tool_calls);
        response = retryResp;
        response.tool_calls = retryNorm.normalized;
        log.info('continue', 'Tool-echo retry succeeded', { tool_calls: response.tool_calls.length, content_len: (response.content || '').length });
      }
    } catch (retryErr) {
      log.warn('continue', 'Tool-echo retry failed', { error: retryErr.message });
    }
  }

  const shouldForceAutoRetry =
    toolsEnabled &&
    shouldAutoRunTasks(executionMode) &&
    response.tool_calls.length === 0 &&
    response.finish_reason === 'stop' &&
    !!planProgress &&
    planProgress.pending_tasks > 0;

  if (shouldForceAutoRetry) {
    log.warn('continue', 'Auto mode stop while plan still has pending tasks, forcing retry', {
      pending_tasks: planProgress.pending_tasks,
    });
    try {
      const retryRaw = await raisin.ai.completion({
        messages: [...history,
          { role: 'assistant', content: response.content || '' },
          { role: 'system', content: 'The plan still has pending tasks. Continue by calling the next required tool now. Do not stop yet.' },
        ],
        model: modelId,
        temperature: agent.properties.temperature,
        tools: toolDefinitions,
        stream: false,
        conversation_path: chatPath,
        conversation_channel: streamChannel || undefined,
      });
      const retryResp = normalizeCompletionResponse(retryRaw);
      const retryNorm = normalizeToolCalls(retryResp.tool_calls);
      response = retryResp;
      response.tool_calls = retryNorm.normalized;
    } catch (retryErr) {
      log.warn('continue', 'Forced auto retry failed', { error: retryErr.message });
    }
  }

  // Pre-scan for plan approval requirement
  let effectiveFinishReason = response.finish_reason;
  if (response.tool_calls.length > 0 && hasPlanningTools) {
    const needsApproval = requiresPlanApproval(executionMode);
    // Only create-plan triggers approval — not update-task, get-plan-status, etc.
    if (needsApproval && response.tool_calls.some(tc => {
      const n = (getToolCallName(tc) || '').replace(/-/g, '_');
      return n === 'create_plan';
    })) {
      effectiveFinishReason = 'awaiting_plan_approval';
    }
  }

  const stepByStepPause =
    shouldPauseAfterTask(executionMode) &&
    !!planProgress &&
    planProgress.task_status === 'completed' &&
    planProgress.pending_tasks > 0;
  if (stepByStepPause) {
    response.tool_calls = [];
    if (!response.content || !response.content.trim()) {
      response.content = 'Completed one task. Waiting for your instruction to continue with the next task.';
    }
    effectiveFinishReason = 'awaiting_step_continue';
  }

  // ── Step 6: Create continuation message ──
  const senderId = outboxCtx ? outboxCtx.agentUserId : 'ai-assistant';
  const senderName = outboxCtx ? outboxCtx.agentDisplayName : 'AI Assistant';
  const assistantContent = response.content || '';

  // Build diagnostics for trace/debug UI
  const executionDiagnostics = {
    history_length: history.length,
    tools_available: toolDefinitions.length,
    planning_enabled: hasPlanningTools,
    stream_channel: streamChannel || null,
    handler: 'agent-continue-handler',
    timestamp: new Date().toISOString(),
  };

  // Build error details for tool loop detection
  const errorDetails = toolLoopDetected ? {
    type: 'tool_loop',
    looped_tools: loopedToolNames,
  } : undefined;

  let nextMsg;
  try {
    const tx = raisin.nodes.beginTransaction();
    nextMsg = await tx.create(workspace, chatPath, {
      name: continuationMsgName,
      node_type: 'raisin:Message',
      properties: {
        role: 'assistant',
        body: { content: assistantContent, message_text: assistantContent },
        content: assistantContent,
        sender_id: senderId,
        sender_display_name: senderName,
        message_type: 'chat',
        status: 'delivered',
        created_at: new Date().toISOString(),
        tokens: response.usage?.total_tokens,
        model: response.model,
        finish_reason: effectiveFinishReason,
        ...(planActionId ? { plan_action_id: planActionId } : {}),
        ...(queuedOriginalMessagePath ? { queued_original_message_path: queuedOriginalMessagePath } : {}),
        parent_message_path: assistantMsgPath,
        turn_terminal_outbox_sent: false,
        turn_terminal_done_emitted: false,
        turn_waiting_emitted: false,
        dispatch_phase: 'pending',
        orchestration_mode: executionMode,
        orchestration_round: nextCount,
        terminal_reason_internal: null,
        continuation_depth: nextCount,
        execution_diagnostics: executionDiagnostics,
        ...(errorDetails ? { error_details: errorDetails } : {}),
      },
    });
    tx.commit();
    log.step('continue', 6, 6, 'Creating continuation message', { name: continuationMsgName, terminal: effectiveFinishReason });

    await createCostRecord(
      workspace,
      nextMsg.path,
      response,
      agent.properties?.provider,
      log.since(t0AI),
    );

    await emitConversationEvent('conversation:message_saved', {
      type: 'message_saved',
      messagePath: nextMsg.path,
      role: 'assistant',
      timestamp: new Date().toISOString(),
    }, chatPath, streamChannel);
  } catch (createErr) {
    if (createErr.message && createErr.message.includes('already exists')) {
      log.warn('continue', 'Continuation already created by another worker, skipping');
      const dup = await raisin.nodes.get(workspace, `${chatPath}/${continuationMsgName}`);
      await resumeTerminalSideEffects(workspace, chatPath, dup, outboxCtx, streamChannel);
      return;
    }
    throw createErr;
  }

  // ── Dispatch tool calls ──

  try {

  if (response.tool_calls.length > 0) {
    for (let i = 0; i < response.tool_calls.length; i++) {
      const tc = response.tool_calls[i];
      const toolName = getToolCallName(tc);
      if (!toolName) {
        const msg = `Malformed tool call payload from model: ${safeJson(tc)}`;
        await emitAssistantTurnError(workspace, chatPath, continuationMsgName, msg, outboxCtx, streamChannel);
        throw new Error(msg);
      }

      const toolCallId = tc.id || `generated-${i}`;
      const toolCallName = tc.id ? `tool-call-${tc.id}` : `tool-call-idx-${i}`;
      const toolRef = toolNameToRef[toolName];

      // ── Unknown tool handling ──
      if (!toolRef) {
        log.warn('continue', 'Unknown tool requested by model', { name: toolName });

        // Check if previous turn also had unknown tools — if so, give up
        const prevChildren = await raisin.nodes.getChildren(workspace, assistantMsgPath);
        const repeatedUnknown = (prevChildren || []).some(c =>
          c.node_type === 'raisin:AIToolCall' &&
          c.properties?.status === 'completed' &&
          c.properties?.function_name &&
          !toolNameToRef[c.properties.function_name]
        );

        if (repeatedUnknown) {
          log.error('continue', 'Repeated unknown tool in continuation', { name: toolName });
          await emitAssistantTurnError(workspace, chatPath, continuationMsgName,
            `The AI model repeatedly requested a tool ("${toolName}") that doesn't exist.`, outboxCtx, streamChannel);
          return;
        }

        const available = Object.keys(toolNameToRef).join(', ');
        await createErrorToolResult(workspace, nextMsg.path, toolCallName, toolCallId, toolName,
          { error: `Tool "${toolName}" does not exist. Available tools: ${available}` });
        continuationExpected = true;
        continue;
      }

      // ── Parse arguments ──
      let toolArgs;
      try {
        toolArgs = parseToolArguments(tc);
      } catch (argsErr) {
        log.warn('continue', 'Invalid arguments for tool', { name: toolName, error: argsErr.message });
        await createErrorToolResult(workspace, nextMsg.path, toolCallName, toolCallId, toolName,
          { error: `Invalid arguments for "${toolName}": ${argsErr.message}` });
        continuationExpected = true;
        continue;
      }

      // Always async dispatch for AI orchestration
      const agentNameFromPath = chatPath.split('/')[2];
      toolArgs.__raisin_context = {
        ...(toolArgs.__raisin_context || {}),
        workspace,
        chat_path: chatPath,
        msg_path: nextMsg.path,
        execution_mode: executionMode,
        agent_name: agentNameFromPath,
        sender_id: outboxCtx?.senderId || null,
        conversation_path: chatPath,
        orchestration_mode: executionMode,
        orchestration_round: nextCount,
      };

      log.info('continue', 'Creating pending tool call', { name: toolName, path: toolRef['raisin:path'] });
      await raisin.nodes.create(workspace, nextMsg.path, {
        name: toolCallName,
        node_type: 'raisin:AIToolCall',
        properties: {
          tool_call_id: toolCallId,
          function_name: toolName,
          function_ref: toolRef,
          arguments: toolArgs,
          status: 'pending',
        },
      });
      pendingToolCount++;

      await emitConversationEvent('conversation:tool_call_started', {
        type: 'tool_call_started',
        toolCallId: toolCallId,
        functionName: toolName,
        arguments: toolArgs,
        timestamp: new Date().toISOString(),
      }, chatPath, streamChannel);
    }

    if (pendingToolCount > 0) {
      log.info('continue', 'Created tool calls', { count: pendingToolCount });
    }
  }

  // ── Thought nodes ──
  if (agent.properties.thinking_enabled && response.thinking) {
    for (let i = 0; i < response.thinking.length; i++) {
      const thought = response.thinking[i];
      await raisin.nodes.create(workspace, nextMsg.path, {
        name: `thought-${i}`,
        node_type: 'raisin:AIThought',
        properties: { content: thought, thought_type: 'reasoning' },
      });
      if (outboxCtx) {
        await sendAgentOutboxMessage(workspace, outboxCtx, thought, 'ai_thought', { thought_type: 'reasoning' });
      }
    }
  }

  // ── Terminal side effects ──
  isWaiting =
    effectiveFinishReason === 'awaiting_plan_approval' ||
    effectiveFinishReason === 'awaiting_step_continue';
  const isTerminal = pendingToolCount === 0 && !continuationExpected && !isWaiting;

  let dispatchPhase = 'pending';
  if (isWaiting || isTerminal) {
    dispatchPhase = 'terminal';
  } else if (pendingToolCount > 0) {
    dispatchPhase = 'awaiting_results';
  } else if (continuationExpected) {
    dispatchPhase = 'queued';
  }

  await updateOrchestrationState(workspace, nextMsg.path, {
    dispatch_phase: dispatchPhase,
    terminal_reason_internal: isTerminal
      ? (effectiveFinishReason || 'stop')
      : (isWaiting ? effectiveFinishReason : (dispatchPhase === 'awaiting_results' ? 'awaiting_results' : null)),
  });

  if (outboxCtx && isTerminal) {
    const content = response.content || TERMINAL_FALLBACK_TEXT;
    const isToolEcho = /^Calling\s+[\w-]+\s*$/.test(content.trim());
    if (!isToolEcho) {
      await sendAgentOutboxMessage(workspace, outboxCtx, content, 'chat', {
        model: response.model,
        finish_reason: effectiveFinishReason,
        tokens: response.usage?.total_tokens,
      }, {
        dedupe_key: `chat_terminal:${nextMsg.path}`,
      });
    }
    await setTerminalMarker(workspace, nextMsg.path, 'turn_terminal_outbox_sent', true);
    log.info('continue', 'Turn complete', { finish_reason: effectiveFinishReason, terminal_event: 'outbox_sent' });
  }

  if (isWaiting) {
    await emitConversationEvent('conversation:waiting', {
      type: 'waiting',
      reason: effectiveFinishReason === 'awaiting_plan_approval' ? 'awaiting_plan_approval' : 'step_by_step',
      dispatchPhase: 'terminal',
      timestamp: new Date().toISOString(),
    }, chatPath, streamChannel);
    await setTerminalMarker(workspace, nextMsg.path, 'turn_waiting_emitted', true);
    terminalEventEmitted = true;
  } else if (isTerminal) {
    await emitConversationEvent('conversation:done', {
      type: 'done',
      content: response.content || TERMINAL_FALLBACK_TEXT,
      role: 'assistant',
      senderDisplayName: senderName,
      finishReason: effectiveFinishReason,
      dispatchPhase: 'terminal',
      terminalReasonInternal: effectiveFinishReason || 'stop',
      timestamp: new Date().toISOString(),
    }, chatPath, streamChannel);
    await setTerminalMarker(workspace, nextMsg.path, 'turn_terminal_done_emitted', true);
    terminalEventEmitted = true;
    await drainQueuedUserIntent(workspace, chatPath, nextMsg.path);
  }

  } catch (toolErr) {
    log.error('continue', 'Error during tool processing', { error: toolErr.message });
    if (queuedOriginalMessagePath) {
      await markQueuedMessageState(workspace, queuedOriginalMessagePath, 'failed_replay', {
        orchestration_queue_error: String(toolErr?.message || toolErr),
      });
    }
    // Ensure the frontend always gets a terminal event
    try {
      await emitConversationEvent('conversation:done', {
        type: 'done',
        content: `Error: ${toolErr.message}`,
        role: 'assistant',
        senderDisplayName: senderName,
        finishReason: 'error',
        timestamp: new Date().toISOString(),
      }, chatPath, streamChannel);
      terminalEventEmitted = true;
    } catch (_) { /* best-effort */ }
    throw toolErr;
  }

  } finally {
    if (!terminalEventEmitted && streamChannel && pendingToolCount === 0 && !continuationExpected && !isWaiting) {
      try {
        log.warn('continue', 'Safety-net: emitting recovered done event');
        await emitConversationEvent('conversation:done', {
          type: 'done',
          content: '',
          role: 'assistant',
          finishReason: 'error',
          recovered: true,
          timestamp: new Date().toISOString(),
        }, chatPath, streamChannel);
      } catch (_) { /* best-effort */ }
    }
  }
}

export { handleToolResult };
