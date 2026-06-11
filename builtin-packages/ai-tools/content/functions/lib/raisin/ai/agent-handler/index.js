/**
 * Agent Handler — AI response generation for agent conversations
 *
 * Triggered by process-agent-chat when a raisin:Message is created in an
 * agent's inbox with message_type="chat" and status="delivered".
 *
 * Flow:
 *   1. Parse trigger input → message path, chat path, reply name
 *   2. Validate inbound user turn (skip agent echoes, intermediates)
 *   3. Mark inbox message as read
 *   4. Idempotency: skip if reply exists, resume terminal side-effects
 *   5. Load agent config from functions workspace via agent_ref
 *   6. Resolve tools in parallel, filter planning if disabled
 *   7. Build system prompt + planning additions + user memory
 *   8. Build conversation history (DESCENDANT_OF SQL via shared module)
 *   9. Call AI completion with streaming
 *  10. Create assistant message (transaction, deterministic name)
 *  11. Process tool calls (always async dispatch via raisin:AIToolCall)
 *  12. Create thought nodes if thinking_enabled
 *  13. Terminal side-effects (outbox delivery, SSE done/waiting)
 */

import { log, setContext } from '../agent-shared/logger.js';
import { buildHistoryFromChat } from '../agent-shared/history.js';
import { maybeCompactConversation } from '../agent-shared/compaction.js';
import {
  resolveToolsParallel,
  normalizeToolCalls,
  normalizeCompletionResponse,
  parseToolArguments,
  getToolCallName,
} from '../agent-shared/tools.js';
import {
  resolveAgentOutboxContext,
  sendAgentOutboxMessage,
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
  createCostRecord,
  TERMINAL_FALLBACK_TEXT,
  getPlanningSystemPromptAddition,
  getEffectiveExecutionMode,
  requiresPlanApproval,
  shouldAutoRunTasks,
  updateOrchestrationState,
} from '../agent-shared/utils.js';

// ─── Entry Point ────────────────────────────────────────────────────────────

export async function handleUserMessage(context) {
  const input = context.flow_input ?? context;
  const workspace = input.workspace ?? 'ai';

  const event = input.event ?? (input.node ? {
    type: 'Created',
    node_id: input.node.id,
    node_type: input.node.node_type,
    node_path: input.node.path,
  } : null);

  if (!event?.node_path) {
    console.error('[agent-handler] Missing event or node_path');
    return;
  }

  const messagePath = event.node_path;
  const msgName = messagePath.split('/').pop();
  const chatPath = messagePath.split('/').slice(0, -1).join('/');
  const replyName = `reply-to-${msgName}`;

  if (!chatPath.startsWith('/agents/')) {
    log.debug('handler', 'Skipping non-agent conversation path', { chat: chatPath });
    return;
  }

  setContext({ chat: chatPath });
  log.info('handler', 'Triggered', { msg: messagePath, workspace });

  // ── 1. Fetch and validate trigger message ─────────────────────────────────

  const message = (input.node?.path === messagePath)
    ? input.node
    : await raisin.nodes.get(workspace, messagePath);

  if (!message) {
    log.warn('handler', 'Message not found', { path: messagePath });
    return;
  }

  if (!isInboundUserTurn(message)) {
    log.debug('handler', 'Skipping: not an inbound user turn');
    return;
  }

  // ── 2. Mark message as read ───────────────────────────────────────────────

  await markAsRead(workspace, chatPath, message);

  // ── 3. Idempotency checks ────────────────────────────────────────────────

  const planActionId = message.properties?.plan_action_id || null;

  if (planActionId) {
    const existing = await findExistingPlanReply(workspace, chatPath, planActionId);
    if (existing) {
      log.debug('handler', 'Plan action already handled', { plan_action_id: planActionId });
      const chat = await raisin.nodes.get(workspace, chatPath);
      const channel = resolveStreamChannel(chatPath, chat);
      const outbox = await resolveAgentOutboxContext(workspace, chatPath, chat);
      await resumeTerminalSideEffects(workspace, chatPath, existing, outbox, channel);
      if (existing.properties?.dispatch_phase === 'terminal' && existing.properties?.finish_reason !== 'awaiting_plan_approval') {
        await drainQueuedUserIntent(workspace, chatPath, existing.path);
      }
      return;
    }
  }

  const chat = await raisin.nodes.get(workspace, chatPath);
  if (!chat?.properties?.agent_ref) {
    throw new Error(`Chat not found or missing agent_ref: ${chatPath}`);
  }

  const streamChannel = resolveStreamChannel(chatPath, chat);
  setContext({ channel: streamChannel, chat: chatPath });
  const outboxCtx = await resolveAgentOutboxContext(workspace, chatPath, chat);

  let terminalEventEmitted = false;
  let continuationExpected = false;
  let pendingToolCount = 0;
  const queuedOriginalMessagePath =
    typeof message.properties?.queued_original_message_path === 'string'
      ? message.properties.queued_original_message_path
      : null;
  const isQueuedReplayMessage = Boolean(
    message.properties?.is_system_generated === true && queuedOriginalMessagePath,
  );
  try {

  const existingReply = await raisin.nodes.get(workspace, `${chatPath}/${replyName}`);
  if (existingReply) {
    log.debug('handler', 'Reply already exists', { name: replyName });
    await resumeTerminalSideEffects(workspace, chatPath, existingReply, outboxCtx, streamChannel);
    if (existingReply.properties?.dispatch_phase === 'terminal' && existingReply.properties?.finish_reason !== 'awaiting_plan_approval') {
      await drainQueuedUserIntent(workspace, chatPath, existingReply.path);
    }
    terminalEventEmitted = true;
    return;
  }

  // ── 4. Load agent configuration ──────────────────────────────────────────

  const agentRef = chat.properties.agent_ref;
  const agentPath = typeof agentRef === 'string' ? agentRef : agentRef['raisin:path'];
  const agentWorkspace = typeof agentRef === 'object'
    ? (agentRef['raisin:workspace'] ?? 'functions')
    : 'functions';

  const agent = await raisin.nodes.get(agentWorkspace, agentPath);
  if (!agent) throw new Error(`Agent not found: ${agentPath}`);

  const agentProps = agent.properties ?? {};
  const executionMode = getEffectiveExecutionMode(agentProps.execution_mode);
  log.step('handler', 1, 6, 'Loaded agent config', { path: agentPath });

  // Deterministic orchestration guard:
  // If a previous assistant turn is still running, do not start a parallel branch
  // from a regular user message.
  const systemPlanAction = Boolean(
    message.properties?.plan_action_id
      || message.properties?.is_system_generated
      || message.properties?.sender_id === 'system',
  );
  if (!systemPlanAction) {
    const inFlightTurn = await findInFlightAssistantTurn(workspace, chatPath);
    if (inFlightTurn) {
      await queueUserMessageForReplay(workspace, message.path);
      log.info('handler', 'Queued inbound user message during in-flight turn', {
        message_path: message.path,
        in_flight_turn: inFlightTurn.path,
      });
      return;
    }
  }

  if (executionMode === 'manual') {
    const pendingPlan = await findLatestManualPlanWithPendingTasks(workspace, chatPath);
    const userText = extractMessageText(message);
    if (pendingPlan && isGenericManualContinue(userText)) {
      const content = buildManualTaskSelectionPrompt(pendingPlan);
      let assistantMsg;
      try {
        assistantMsg = await raisin.nodes.create(workspace, chatPath, {
          name: replyName,
          node_type: 'raisin:Message',
          properties: {
            role: 'assistant',
            body: { content, message_text: content },
            content,
            sender_id: outboxCtx?.agentUserId ?? 'ai-assistant',
            sender_display_name: outboxCtx?.agentDisplayName ?? 'AI Assistant',
            message_type: 'chat',
            status: 'delivered',
            created_at: new Date().toISOString(),
            finish_reason: 'stop',
            parent_message_path: messagePath,
            dispatch_phase: 'terminal',
            orchestration_mode: executionMode,
            orchestration_round: 0,
            terminal_reason_internal: 'manual_task_selection_required',
            turn_terminal_outbox_sent: false,
            turn_terminal_done_emitted: false,
            turn_waiting_emitted: false,
          },
        });
      } catch (err) {
        if (!String(err?.message || '').includes('already exists')) throw err;
        assistantMsg = await raisin.nodes.get(workspace, `${chatPath}/${replyName}`);
      }
      if (!assistantMsg) {
        log.warn('handler', 'Manual clarification message missing after create race', { replyName });
        return;
      }

      await emitConversationEvent('conversation:message_saved', {
        type: 'message_saved',
        messagePath: assistantMsg.path,
        role: 'assistant',
        timestamp: new Date().toISOString(),
      }, chatPath, streamChannel);

      if (outboxCtx) {
        await sendAgentOutboxMessage(workspace, outboxCtx, content, 'chat', {
          finish_reason: 'stop',
        }, {
          dedupe_key: `chat_terminal:${assistantMsg.path}`,
        });
        await setTerminalMarker(workspace, assistantMsg.path, 'turn_terminal_outbox_sent', true);
      }

      await emitConversationEvent('conversation:done', {
        type: 'done',
        content,
        role: 'assistant',
        senderDisplayName: outboxCtx?.agentDisplayName ?? 'AI Assistant',
        finishReason: 'stop',
        dispatchPhase: 'terminal',
        terminalReasonInternal: 'manual_task_selection_required',
        timestamp: new Date().toISOString(),
      }, chatPath, streamChannel);
      await setTerminalMarker(workspace, assistantMsg.path, 'turn_terminal_done_emitted', true);
      terminalEventEmitted = true;
      return;
    }
  }

  // ── 4b. Conversation token budget guard ──────────────────────────────────
  // When max_conversation_tokens is set on the agent and the running total
  // (total_tokens_used, maintained by createCostRecord) has reached it,
  // refuse the turn without calling the AI.

  const maxConversationTokens = Number(agentProps.max_conversation_tokens) || 0;
  if (maxConversationTokens > 0) {
    const tokensUsed = Number(chat.properties?.total_tokens_used) || 0;
    if (tokensUsed >= maxConversationTokens) {
      log.warn('handler', 'Conversation token budget exceeded', {
        used: tokensUsed,
        limit: maxConversationTokens,
      });
      const content = `This conversation has reached its token budget (${tokensUsed} used / ${maxConversationTokens} limit). Please start a new conversation.`;
      let budgetMsg;
      try {
        budgetMsg = await raisin.nodes.create(workspace, chatPath, {
          name: replyName,
          node_type: 'raisin:Message',
          properties: {
            role: 'assistant',
            body: { content, message_text: content },
            content,
            sender_id: outboxCtx?.agentUserId ?? 'ai-assistant',
            sender_display_name: outboxCtx?.agentDisplayName ?? 'AI Assistant',
            message_type: 'chat',
            status: 'delivered',
            created_at: new Date().toISOString(),
            finish_reason: 'budget_exceeded',
            parent_message_path: messagePath,
            dispatch_phase: 'terminal',
            orchestration_mode: executionMode,
            orchestration_round: 0,
            terminal_reason_internal: 'budget_exceeded',
            turn_terminal_outbox_sent: false,
            turn_terminal_done_emitted: false,
            turn_waiting_emitted: false,
          },
        });
      } catch (err) {
        if (!String(err?.message || '').includes('already exists')) throw err;
        budgetMsg = await raisin.nodes.get(workspace, `${chatPath}/${replyName}`);
      }
      if (!budgetMsg) {
        log.warn('handler', 'Budget message missing after create race', { replyName });
        return;
      }

      await emitConversationEvent('conversation:message_saved', {
        type: 'message_saved',
        messagePath: budgetMsg.path,
        role: 'assistant',
        timestamp: new Date().toISOString(),
      }, chatPath, streamChannel);

      if (outboxCtx) {
        await sendAgentOutboxMessage(workspace, outboxCtx, content, 'chat', {
          finish_reason: 'budget_exceeded',
        }, {
          dedupe_key: `chat_terminal:${budgetMsg.path}`,
        });
        await setTerminalMarker(workspace, budgetMsg.path, 'turn_terminal_outbox_sent', true);
      }

      await emitConversationEvent('conversation:done', {
        type: 'done',
        content,
        role: 'assistant',
        senderDisplayName: outboxCtx?.agentDisplayName ?? 'AI Assistant',
        finishReason: 'budget_exceeded',
        dispatchPhase: 'terminal',
        terminalReasonInternal: 'budget_exceeded',
        timestamp: new Date().toISOString(),
      }, chatPath, streamChannel);
      await setTerminalMarker(workspace, budgetMsg.path, 'turn_terminal_done_emitted', true);
      terminalEventEmitted = true;
      return;
    }
  }

  // ── 5. Resolve tools ─────────────────────────────────────────────────────

  let { toolDefinitions, toolNameToRef } = await resolveToolsParallel(agentProps.tools || []);

  const taskCreationEnabled = agentProps.task_creation_enabled === true;
  if (!taskCreationEnabled) {
    const planningNames = Object.entries(toolNameToRef)
      .filter(([, ref]) => ref.category === 'planning')
      .map(([name]) => name);
    if (planningNames.length > 0) {
      toolDefinitions = toolDefinitions.filter(td => !planningNames.includes(td.function?.name));
      for (const name of planningNames) delete toolNameToRef[name];
    }
  }

  const hasPlanningTools = taskCreationEnabled
    && Object.values(toolNameToRef).some(r => r.category === 'planning');
  log.step('handler', 2, 6, 'Resolved tools', { count: toolDefinitions.length, planning: hasPlanningTools });

  // ── 6. Build system prompt ────────────────────────────────────────────────

  let systemPrompt = agentProps.system_prompt || '';

  if (hasPlanningTools) {
    systemPrompt += '\n' + getPlanningSystemPromptAddition(executionMode);
  }

  if (outboxCtx) {
    const agentName = chatPath.split('/')[2] || null;
    const memory = await loadUserMemory(agentName, outboxCtx.senderId);
    if (memory) systemPrompt += formatMemoryForPrompt(memory);
  }

  // Inject agent rules
  const rules = agentProps.rules;
  if (Array.isArray(rules) && rules.length > 0) {
    systemPrompt += '\n\n## Rules\n' + rules.map(r => `- ${r}`).join('\n');
  }

  // ── 7. Build history and call AI completion ───────────────────────────────

  const modelId = agentProps.provider
    ? `${agentProps.provider}:${agentProps.model}`
    : agentProps.model;

  // Auto-compaction: summarize older messages once the threshold is crossed.
  // The persisted raisin:AICompaction node is picked up by the history builder.
  await maybeCompactConversation(workspace, chatPath, agentProps, modelId);

  const history = await buildHistoryFromChat(workspace, chatPath, systemPrompt, null, null, {
    maxHistoryMessages: agentProps.max_history_messages,
  });
  log.step('handler', 3, 6, 'Built history', { entries: history.length });

  let response;
  const t0AI = log.time();
  try {
    const raw = await raisin.ai.completion({
      messages: history,
      model: modelId,
      temperature: agentProps.temperature,
      tools: toolDefinitions.length > 0 ? toolDefinitions : undefined,
      stream: true,
      conversation_path: chatPath,
      conversation_channel: streamChannel || undefined,
    });
    response = normalizeCompletionResponse(raw);
  } catch (err) {
    log.error('handler', 'AI completion failed', { error: err.message });
    await emitAssistantTurnError(workspace, chatPath, replyName, err.message, outboxCtx, streamChannel);
    terminalEventEmitted = true;
    throw err;
  }

  log.step('handler', 4, 6, 'AI response received', {
    finish: response.finish_reason,
    tools: response.tool_calls.length,
  });

  // Normalize tool calls and handle malformed entries
  const normalized = normalizeToolCalls(response.tool_calls);
  response.tool_calls = normalized.normalized;

  if (normalized.malformed.length > 0) {
    log.warn('handler', 'Dropped malformed tool calls', {
      count: normalized.malformed.length,
      entries: safeJson(normalized.malformed),
    });
    if (!response.tool_calls.length && !response.content.trim()) {
      response.content = 'I received an invalid tool call from the model. Please try again.';
      response.finish_reason = response.finish_reason || 'stop';
    }
  }

  // Detect raw function syntax in content (model quirk, e.g. Llama)
  if (response.tool_calls.length === 0 && /<function=[\w-]+>/.test(response.content || '')) {
    log.warn('handler', 'Model emitted raw <function=...> syntax, retrying without tools');
    try {
      const retryRaw = await raisin.ai.completion({
        messages: [...history,
          { role: 'assistant', content: response.content },
          { role: 'user', content: 'Your previous response used invalid function call syntax. Respond in plain text.' },
        ],
        model: modelId,
        temperature: agentProps.temperature,
        tools: undefined,
        stream: false,
        conversation_path: chatPath,
        conversation_channel: streamChannel || undefined,
      });
      const retryResp = normalizeCompletionResponse(retryRaw);
      if (retryResp.content?.trim() && !/<function=[\w-]+>/.test(retryResp.content)) {
        response.content = retryResp.content;
        log.info('handler', 'Retry after raw function syntax succeeded');
      }
    } catch (retryErr) {
      log.warn('handler', 'Retry after raw function syntax failed', { error: retryErr.message });
    }
    if (/<function=[\w-]+>/.test(response.content || '')) {
      response.content = (response.content || '').replace(/<function=[\w-]+>[\s\S]*?<\/function>/g, '').trim();
    }
  }

  // Detect tool-echo: model wrote "Calling update-task" as text instead of making the call.
  if (toolDefinitions.length > 0 && response.tool_calls.length === 0 && /^Calling\s+[\w-]+/i.test((response.content || '').trim())) {
    log.warn('handler', 'Model echoed tool name as text instead of calling it, retrying with nudge', { content: response.content.trim() });
    try {
      const retryRaw = await raisin.ai.completion({
        messages: [...history,
          { role: 'assistant', content: response.content },
          { role: 'system', content: 'You wrote the tool name as plain text instead of actually calling it. You MUST use the function calling mechanism to invoke tools. Do NOT write "Calling ..." as text. Actually call the function now and continue executing the plan.' },
        ],
        model: modelId,
        temperature: agentProps.temperature,
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
        log.info('handler', 'Tool-echo retry succeeded', { tool_calls: response.tool_calls.length, content_len: (response.content || '').length });
      }
    } catch (retryErr) {
      log.warn('handler', 'Tool-echo retry failed', { error: retryErr.message });
    }
  }

  // In auto modes, do not allow the first post-approval turn to stop without tools.
  if (
    planActionId &&
    shouldAutoRunTasks(executionMode) &&
    toolDefinitions.length > 0 &&
    response.tool_calls.length === 0 &&
    response.finish_reason === 'stop'
  ) {
    log.warn('handler', 'Auto mode stop without tool calls after approval, forcing one retry');
    try {
      const retryRaw = await raisin.ai.completion({
        messages: [...history,
          { role: 'assistant', content: response.content || '' },
          { role: 'system', content: 'Plan execution is in auto mode. Continue the plan by calling tools now. Do not stop until at least one tool call is issued.' },
        ],
        model: modelId,
        temperature: agentProps.temperature,
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
      log.warn('handler', 'Auto-mode retry failed', { error: retryErr.message });
    }
  }

  if (!response.tool_calls.length && !response.content.trim()) {
    // If this is a plan approval continuation, retry once with an explicit nudge
    if (planActionId && toolDefinitions.length > 0) {
      log.warn('handler', 'Empty response on plan continuation, retrying with hint');
      const retryHistory = [
        ...history,
        { role: 'assistant', content: '' },
        { role: 'user', content: 'You returned an empty response. You MUST respond now. Call update_task to start the first pending task, then execute it using the available tools.' },
      ];
      try {
        const retryRaw = await raisin.ai.completion({
          messages: retryHistory,
          model: modelId,
          temperature: agentProps.temperature,
          tools: toolDefinitions.length > 0 ? toolDefinitions : undefined,
          stream: false,
          conversation_path: chatPath,
          conversation_channel: streamChannel || undefined,
        });
        const retryResp = normalizeCompletionResponse(retryRaw);
        if (retryResp.content?.trim() || retryResp.tool_calls?.length > 0) {
          response = retryResp;
          const retryNorm = normalizeToolCalls(response.tool_calls);
          response.tool_calls = retryNorm.normalized;
          log.info('handler', 'Retry succeeded', { content_len: response.content.length, tools: response.tool_calls.length });
        }
      } catch (retryErr) {
        log.warn('handler', 'Retry also failed', { error: retryErr.message });
      }
    }
    // Final fallback if retry didn't help or wasn't applicable
    if (!response.tool_calls.length && !response.content.trim()) {
      log.warn('handler', 'AI returned empty response', {
        finish_reason: response.finish_reason,
        content_len: response.content?.length || 0,
        tool_calls_len: response.tool_calls?.length || 0,
        model: response.model,
        history_entries: history.length,
        plan_action_id: planActionId,
      });
      response.content = TERMINAL_FALLBACK_TEXT;
      response.finish_reason = response.finish_reason || 'stop';
    }
  }

  // Pre-scan: detect if plan approval is needed
  let effectiveFinishReason = response.finish_reason;
  if (hasPlanningTools && response.tool_calls.length > 0) {
    const needsApproval = requiresPlanApproval(executionMode);
    if (needsApproval) {
      // Only create-plan triggers approval — not update-task, get-plan-status, etc.
      const hasPlanCreation = response.tool_calls.some(tc => {
        const name = (getToolCallName(tc) || '').replace(/-/g, '_');
        return name === 'create_plan';
      });
      if (hasPlanCreation) effectiveFinishReason = 'awaiting_plan_approval';
    }
  }

  // ── 8. Create assistant message (transaction + deterministic name) ────────

  const senderId = outboxCtx?.agentUserId ?? 'ai-assistant';
  const senderName = outboxCtx?.agentDisplayName ?? 'AI Assistant';

  // Build diagnostics for trace/debug UI
  const executionDiagnostics = {
    history_length: history.length,
    tools_available: toolDefinitions.length,
    planning_enabled: hasPlanningTools,
    stream_channel: streamChannel || null,
    handler: 'agent-handler',
    timestamp: new Date().toISOString(),
  };

  const isFallbackContent = response.content === TERMINAL_FALLBACK_TEXT;
  const errorDetails = isFallbackContent ? {
    type: 'empty_response',
    finish_reason: response.finish_reason,
    model: response.model,
    plan_action_id: planActionId,
  } : undefined;

  let assistantMsg;
  try {
    const tx = raisin.nodes.beginTransaction();
    assistantMsg = await tx.create(workspace, chatPath, {
      name: replyName,
      node_type: 'raisin:Message',
      properties: {
        role: 'assistant',
        body: { content: response.content, message_text: response.content },
        content: response.content,
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
        parent_message_path: messagePath,
        turn_terminal_outbox_sent: false,
        turn_terminal_done_emitted: false,
        turn_waiting_emitted: false,
        dispatch_phase: 'pending',
        orchestration_mode: executionMode,
        orchestration_round: 0,
        terminal_reason_internal: null,
        execution_diagnostics: executionDiagnostics,
        ...(errorDetails ? { error_details: errorDetails } : {}),
      },
    });
    tx.commit();
  } catch (err) {
    if (String(err?.message ?? '').includes('already exists')) {
      log.warn('handler', 'Reply created by another worker');
      const existing = await raisin.nodes.get(workspace, `${chatPath}/${replyName}`);
      await resumeTerminalSideEffects(workspace, chatPath, existing, outboxCtx, streamChannel);
      return;
    }
    throw err;
  }

  await emitConversationEvent('conversation:message_saved', {
    type: 'message_saved',
    messagePath: assistantMsg.path,
    role: 'assistant',
    timestamp: new Date().toISOString(),
  }, chatPath, streamChannel);

  await createCostRecord(
    workspace,
    chatPath,
    assistantMsg.path,
    response,
    agentProps.provider,
    log.since(t0AI),
  );

  log.step('handler', 5, 6, 'Created assistant message', { path: assistantMsg.path });

  // ── 9. Process tool calls ─────────────────────────────────────────────────

  try {

  for (let i = 0; i < response.tool_calls.length; i++) {
    const tc = response.tool_calls[i];
    const toolName = getToolCallName(tc);
    const callId = tc.id || `generated-${i}`;
    const callNodeName = tc.id ? `tool-call-${tc.id}` : `tool-call-idx-${i}`;

    if (!toolName) {
      log.error('handler', 'Malformed tool call', { payload: safeJson(tc) });
      await emitAssistantTurnError(
        workspace, chatPath, replyName,
        `Malformed tool call: ${safeJson(tc)}`,
        outboxCtx, streamChannel,
      );
      throw new Error('Malformed tool call from model');
    }

    const toolRef = toolNameToRef[toolName];

    // Unknown tool → create error result, let model retry once
    if (!toolRef) {
      log.warn('handler', 'Unknown tool requested', { name: toolName });

      const isRetry = await isRepeatedUnknownTool(workspace, message.path, toolNameToRef);
      if (isRetry) {
        await emitAssistantTurnError(
          workspace, chatPath, replyName,
          `The model repeatedly requested unknown tool "${toolName}".`,
          outboxCtx, streamChannel,
        );
        return;
      }

      const available = Object.keys(toolNameToRef).join(', ');
      await createErrorToolResult(workspace, assistantMsg.path, callNodeName, callId, toolName, {
        error: `Tool "${toolName}" does not exist. Available: ${available}`,
      });
      continuationExpected = true;
      continue;
    }

    // Parse tool arguments
    let toolArgs;
    try {
      toolArgs = parseToolArguments(tc);
    } catch (err) {
      log.warn('handler', 'Invalid tool arguments', { name: toolName, error: err.message });
      await createErrorToolResult(workspace, assistantMsg.path, callNodeName, callId, toolName, {
        error: `Invalid arguments for "${toolName}": ${err.message}`,
      });
      continuationExpected = true;
      continue;
    }

    // Always async dispatch for AI agent orchestration
    log.info('handler', 'Queueing tool call', { name: toolName });

    // Inject execution context so the Rust executor passes it to the function
    toolArgs.__raisin_context = {
      ...(toolArgs.__raisin_context || {}),
      workspace,
      chat_path: chatPath,
      msg_path: assistantMsg.path,
      execution_mode: executionMode,
      agent_name: chatPath.split('/')[2] || null,
      sender_id: outboxCtx?.senderId || null,
      conversation_path: chatPath,
      orchestration_mode: executionMode,
      orchestration_round: 0,
    };

    const callNode = await raisin.nodes.create(workspace, assistantMsg.path, {
      name: callNodeName,
      node_type: 'raisin:AIToolCall',
      properties: {
        tool_call_id: callId,
        function_name: toolName,
        function_ref: toolRef,
        arguments: toolArgs,
        status: 'pending',
      },
    });
    if (callNode?.error && !String(callNode.error).includes('already exists')) {
      throw new Error(`Failed to create tool-call node "${callNodeName}": ${callNode.error}`);
    }
    pendingToolCount++;

    await emitConversationEvent('conversation:tool_call_started', {
      type: 'tool_call_started',
      toolCallId: callId,
      functionName: toolName,
      arguments: toolArgs,
      timestamp: new Date().toISOString(),
    }, chatPath, streamChannel);
  }

  // ── 10. Create thought nodes ──────────────────────────────────────────────

  if (agentProps.thinking_enabled && Array.isArray(response.thinking)) {
    for (let i = 0; i < response.thinking.length; i++) {
      await raisin.nodes.create(workspace, assistantMsg.path, {
        name: `thought-${i}`,
        node_type: 'raisin:AIThought',
        properties: { content: response.thinking[i], thought_type: 'reasoning' },
      });
      if (outboxCtx) {
        await sendAgentOutboxMessage(workspace, outboxCtx, response.thinking[i], 'ai_thought', {
          thought_type: 'reasoning',
        });
      }
    }
  }

  // ── 11. Terminal side-effects ─────────────────────────────────────────────

  let dispatchPhase = 'pending';
  if (effectiveFinishReason === 'awaiting_plan_approval') {
    dispatchPhase = 'terminal';
  } else if (pendingToolCount > 0) {
    dispatchPhase = 'awaiting_results';
  } else if (continuationExpected) {
    dispatchPhase = 'queued';
  } else {
    dispatchPhase = 'terminal';
  }

  const isTerminal = pendingToolCount === 0
    && !continuationExpected
    && effectiveFinishReason !== 'awaiting_plan_approval';

  await updateOrchestrationState(workspace, assistantMsg.path, {
    dispatch_phase: dispatchPhase,
    terminal_reason_internal: isTerminal
      ? (effectiveFinishReason || 'stop')
      : (dispatchPhase === 'awaiting_results' ? 'awaiting_results' : null),
  });

  // Outbox delivery for terminal turns (suppress tool-echo content like "Calling update-task")
  if (isTerminal && outboxCtx) {
    const terminalContent = response.content || TERMINAL_FALLBACK_TEXT;
    const isToolEcho = /^Calling\s+[\w-]+\s*$/.test(terminalContent.trim());
    if (!isToolEcho) {
      await sendAgentOutboxMessage(workspace, outboxCtx, terminalContent, 'chat', {
        model: response.model,
        finish_reason: effectiveFinishReason,
        tokens: response.usage?.total_tokens,
      }, {
        dedupe_key: `chat_terminal:${assistantMsg.path}`,
      });
    }
    await setTerminalMarker(workspace, assistantMsg.path, 'turn_terminal_outbox_sent', true);
  }

  // SSE terminal events
  if (effectiveFinishReason === 'awaiting_plan_approval') {
    await emitConversationEvent('conversation:waiting', {
      type: 'waiting',
      reason: 'awaiting_plan_approval',
      dispatchPhase: 'terminal',
      timestamp: new Date().toISOString(),
    }, chatPath, streamChannel);
    await setTerminalMarker(workspace, assistantMsg.path, 'turn_waiting_emitted', true);
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
    await setTerminalMarker(workspace, assistantMsg.path, 'turn_terminal_done_emitted', true);
    terminalEventEmitted = true;
    await drainQueuedUserIntent(workspace, chatPath, assistantMsg.path);
  }

  log.step('handler', 6, 6, 'Turn complete', {
    finish: effectiveFinishReason,
    terminal: isTerminal,
    dispatch_phase: dispatchPhase,
    tools_queued: pendingToolCount,
  });

  } catch (toolErr) {
    log.error('handler', 'Error during tool processing', { error: toolErr.message });
    if (isQueuedReplayMessage && queuedOriginalMessagePath) {
      await markQueuedMessageState(workspace, queuedOriginalMessagePath, 'failed_replay', {
        orchestration_queue_error: String(toolErr?.message || toolErr),
      });
    }
    // A failed turn must not leave a non-terminal assistant message behind —
    // that would strand the in-flight guard and block the next user message.
    await updateOrchestrationState(workspace, assistantMsg.path, { dispatch_phase: 'terminal' });
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
    if (!terminalEventEmitted && streamChannel && pendingToolCount === 0 && !continuationExpected) {
      try {
        log.warn('handler', 'Safety-net: emitting recovered done event');
        await emitConversationEvent('conversation:done', {
          type: 'done',
          content: '',
          role: 'assistant',
          finishReason: 'error',
          recovered: true,
          timestamp: new Date().toISOString(),
        }, chatPath, streamChannel);
        // Finalize phase so the recovered turn cannot strand the in-flight guard.
        await updateOrchestrationState(workspace, assistantMsg.path, { dispatch_phase: 'terminal' });
      } catch (_) { /* best-effort */ }
    }
  }
}

// ─── Validation ─────────────────────────────────────────────────────────────

function isInboundUserTurn(message) {
  const props = message.properties ?? {};
  const role = props.role;
  const msgType = props.message_type;
  const senderId = typeof props.sender_id === 'string' ? props.sender_id : '';

  const isChatType = msgType === 'chat' || msgType === 'direct_message';
  const isExplicitUser = role === 'user';
  const isAgentSender = senderId.startsWith('agent:');

  // Accept explicit role=user, or chat-type messages without role (delivered from human inbox)
  return (isExplicitUser || (isChatType && !role)) && !isAgentSender;
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

function isGenericManualContinue(text) {
  if (!text) return false;
  const normalized = text.toLowerCase().replace(/[!?.,]/g, '').trim();
  const generic = [
    'go ahead',
    'continue',
    'go on',
    'proceed',
    'next',
    'yes',
    'ok',
    'okay',
    'start',
    'do it',
  ];
  if (generic.includes(normalized)) return true;
  if (/^(go ahead|continue|proceed|go on)\b/.test(normalized) && normalized.split(/\s+/).length <= 3) {
    return true;
  }
  return false;
}

function buildManualTaskSelectionPrompt(plan) {
  const pendingTasks = Array.isArray(plan?.pendingTasks) ? plan.pendingTasks : [];
  const taskLines = pendingTasks
    .slice(0, 20)
    .map((task, index) => `${index + 1}. ${task.title}`)
    .join('\n');
  return `The plan "${plan.title}" is approved and ready.\nIn manual mode, tell me exactly which task to run next.\n\nPending tasks:\n${taskLines}\n\nReply with a specific instruction, for example: "Run task 2".`;
}

async function findLatestManualPlanWithPendingTasks(workspace, chatPath) {
  const plans = await raisin.sql.query(
    `SELECT path, properties
     FROM '${workspace}'
     WHERE DESCENDANT_OF($1)
       AND node_type = 'raisin:AIPlan'
     ORDER BY created_at DESC
     LIMIT 20`,
    [chatPath],
  );
  if (!Array.isArray(plans) || plans.length === 0) return null;

  for (const plan of plans) {
    const status = String(plan?.properties?.status || '');
    if (status === 'pending_approval' || status === 'cancelled' || status === 'completed') {
      continue;
    }

    const taskRows = await raisin.sql.query(
      `SELECT id, properties
       FROM '${workspace}'
       WHERE CHILD_OF($1)
         AND node_type = 'raisin:AITask'
       ORDER BY created_at ASC`,
      [plan.path],
    );

    const pendingTasks = (Array.isArray(taskRows) ? taskRows : [])
      .map(task => ({
        id: task.id,
        title: task?.properties?.title || 'Untitled task',
        status: String(task?.properties?.status || 'pending'),
      }))
      .filter(task => task.status !== 'completed' && task.status !== 'cancelled');

    if (pendingTasks.length > 0) {
      return {
        path: plan.path,
        title: plan?.properties?.title || 'Plan',
        pendingTasks,
      };
    }
  }

  return null;
}

// ─── Inbox Management ───────────────────────────────────────────────────────

async function markAsRead(workspace, chatPath, message) {
  if (workspace !== 'ai' || !message?.path) return;

  try {
    if (message.properties?.status !== 'read') {
      await raisin.nodes.updateProperty(workspace, message.path, 'status', 'read');
      await raisin.nodes.updateProperty(workspace, message.path, 'read_at', new Date().toISOString());
    }
  } catch (e) {
    log.warn('handler', 'Failed to mark as read', { error: e.message });
  }

  try {
    const chatNode = await raisin.nodes.get(workspace, chatPath);
    if (Number(chatNode?.properties?.unread_count) > 0) {
      await raisin.nodes.updateProperty(workspace, chatPath, 'unread_count', 0);
    }
  } catch (e) {
    log.warn('handler', 'Failed to reset unread count', { error: e.message });
  }
}

// ─── Idempotency ────────────────────────────────────────────────────────────

async function findExistingPlanReply(workspace, chatPath, planActionId) {
  const rows = await raisin.sql.query(`
    SELECT path FROM '${workspace}'
    WHERE CHILD_OF($1)
      AND node_type = 'raisin:Message'
      AND properties->>'role'::STRING = 'assistant'
      AND properties->>'plan_action_id'::STRING = $2
    ORDER BY created_at DESC LIMIT 1
  `, [chatPath, planActionId]);

  if (Array.isArray(rows) && rows.length > 0) {
    return raisin.nodes.get(workspace, rows[0].path);
  }
  return null;
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
    log.warn('handler', 'Failed to update queued message state', {
      message_path: messagePath,
      state,
      error: err?.message || String(err),
    });
  }
}

async function queueUserMessageForReplay(workspace, messagePath) {
  if (!workspace || !messagePath) return;
  const existing = await raisin.nodes.get(workspace, messagePath);
  const existingState = String(existing?.properties?.orchestration_queue_state || '');
  if (existingState === 'queued' || existingState === 'replaying' || existingState === 'consumed') {
    return;
  }
  const now = new Date().toISOString();
  const order = Date.now();
  try {
    await raisin.nodes.updateProperty(workspace, messagePath, 'orchestration_queue_state', 'queued');
    await raisin.nodes.updateProperty(workspace, messagePath, 'orchestration_queue_at', now);
    await raisin.nodes.updateProperty(workspace, messagePath, 'orchestration_queue_order', order);
  } catch (err) {
    log.warn('handler', 'Failed to mark message as queued', {
      message_path: messagePath,
      error: err?.message || String(err),
    });
  }
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

  if (!Array.isArray(rows) || rows.length === 0) {
    return null;
  }

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
  const replayBody = {
    content: replayContent,
    message_text: replayContent,
  };

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
        body: replayBody,
        sender_id: queuedProps.sender_id || 'system',
        sender_display_name: queuedProps.sender_display_name || 'System',
        created_at: new Date().toISOString(),
        is_system_generated: true,
        queued_original_message_path: queued.path,
      },
    });
    created = true;
  } catch (err) {
    if (!String(err?.message || '').includes('already exists')) {
      throw err;
    }
  }

  await markQueuedMessageState(workspace, queued.path, 'replaying', {
    queued_replay_message_path: replayPath,
  });

  log.info('handler', 'Queued replay message ready', {
    source_message: queued.path,
    replay_message: replayPath,
    created,
  });
}

const IN_FLIGHT_DISPATCH_PHASES = new Set([
  'pending',
  'queued',
  'awaiting_results',
  'ready_for_model',
]);

// Stale-turn recovery thresholds.
//
// A turn whose continuation event is lost (e.g. the aggregated_result trigger
// never reached agent-continue-handler because of a restart or a dropped
// trigger-evaluation job) stays non-terminal FOREVER, and the in-flight guard
// then queues every subsequent user message — the chat is dead until manual
// intervention. (Production incident chat-5494d4d7: weather tool completed in
// 44ms, aggregated_result persisted, but no continue-3 turn was ever created;
// continue-2 stayed at dispatch_phase=awaiting_results and all later user
// messages were stuck in orchestration_queue_state=queued.)
//
// Tier 1: aggregation already finished (aggregated_result exists) but the
//         continuation never started. The only remaining step is a model call
//         that normally takes seconds — if nothing has progressed within the
//         grace window, the continuation event is gone.
const STALE_TURN_AGGREGATED_GRACE_MS = 2 * 60 * 1000;
// Tier 2: hard ceiling for any in-flight turn. Past the job watchdog's
//         full retry budget nothing legitimate is still running.
const STALE_TURN_MAX_AGE_MS = 30 * 60 * 1000;

function parseNodeTimestamp(node) {
  const raw = node?.properties?.created_at || node?.created_at || '';
  const ts = Date.parse(typeof raw === 'string' ? raw : String(raw));
  return Number.isFinite(ts) ? ts : 0;
}

/**
 * Detect and recover an abandoned in-flight turn. Returns true when the turn
 * was marked terminal (the caller may then proceed with the new user message
 * instead of queueing it forever).
 */
async function recoverStaleInFlightTurn(workspace, latest) {
  const createdAtMs = parseNodeTimestamp(latest);
  const ageMs = createdAtMs > 0 ? Date.now() - createdAtMs : 0;

  let staleReason = null;
  if (createdAtMs > 0 && ageMs > STALE_TURN_MAX_AGE_MS) {
    staleReason = 'stale_turn_max_age';
  } else {
    // Lost-continuation signature: aggregation done, no progress since.
    try {
      const agg = await raisin.nodes.get(workspace, `${latest.path}/aggregated_result`);
      if (agg && !agg.error) {
        const aggAtMs = parseNodeTimestamp(agg) || createdAtMs;
        if (aggAtMs > 0 && Date.now() - aggAtMs > STALE_TURN_AGGREGATED_GRACE_MS) {
          staleReason = 'stale_turn_lost_continuation';
        }
      }
    } catch (_) { /* no aggregated_result — not the lost-continuation case */ }
  }

  if (!staleReason) return false;

  log.warn('handler', 'Recovering abandoned in-flight turn', {
    turn_path: latest.path,
    dispatch_phase: latest?.properties?.dispatch_phase,
    age_ms: ageMs,
    reason: staleReason,
  });
  await updateOrchestrationState(workspace, latest.path, {
    dispatch_phase: 'terminal',
    terminal_reason_internal: staleReason,
  });
  return true;
}

export async function findInFlightAssistantTurn(workspace, chatPath) {
  // A turn is only "in flight" if the LATEST assistant message is still
  // non-terminal. Earlier messages in a finished tool/continuation chain are
  // intentionally left in a non-terminal phase (e.g. 'ready_for_model' on the
  // original tool-dispatching message), so matching *any* non-terminal message
  // would strand the guard forever and queue every subsequent user turn.
  const rows = await raisin.sql.query(`
    SELECT path
    FROM '${workspace}'
    WHERE CHILD_OF($1)
      AND node_type = 'raisin:Message'
      AND properties->>'role'::STRING = 'assistant'
    ORDER BY created_at DESC
    LIMIT 1
  `, [chatPath]);

  if (!Array.isArray(rows) || rows.length === 0) return null;

  const latest = await raisin.nodes.get(workspace, rows[0].path);
  const phase = latest?.properties?.dispatch_phase;
  if (!IN_FLIGHT_DISPATCH_PHASES.has(phase)) return null;

  // Self-heal: a turn abandoned by a lost continuation event must not block
  // the chat forever. If it is recovered (marked terminal), report no
  // in-flight turn so the new user message proceeds normally.
  try {
    if (await recoverStaleInFlightTurn(workspace, latest)) return null;
  } catch (err) {
    log.warn('handler', 'Stale-turn recovery failed; keeping turn in flight', {
      turn_path: latest?.path,
      error: err?.message || String(err),
    });
  }

  return rows[0];
}

// ─── Tool Call Helpers ──────────────────────────────────────────────────────

async function isRepeatedUnknownTool(workspace, parentMsgPath, knownTools) {
  try {
    const children = await raisin.nodes.getChildren(workspace, parentMsgPath);
    return (children || []).some(c =>
      c.node_type === 'raisin:AIToolCall'
      && c.properties?.status === 'completed'
      && c.properties?.function_name
      && !knownTools[c.properties.function_name]
    );
  } catch (_) {
    return false;
  }
}

async function createErrorToolResult(workspace, parentPath, callNodeName, callId, toolName, errorResult) {
  // NOTE: raisin.nodes.create does not throw on backend errors — it returns
  // an { error } object. Check explicitly, otherwise a failed create cascades
  // into "converting from js 'undefined' into type 'string'" on .path access.
  const callNode = await raisin.nodes.create(workspace, parentPath, {
    name: callNodeName,
    node_type: 'raisin:AIToolCall',
    properties: {
      tool_call_id: callId,
      function_name: toolName,
      arguments: {},
      status: 'completed',
    },
  });
  if (callNode?.error) {
    throw new Error(`Failed to create error tool-call node "${callNodeName}": ${callNode.error}`);
  }
  const resultParent = callNode?.path || `${parentPath}/${callNodeName}`;
  const resultNode = await raisin.nodes.create(workspace, resultParent, {
    name: 'result',
    node_type: 'raisin:AIToolSingleCallResult',
    properties: {
      tool_call_id: callId,
      function_name: toolName,
      result: errorResult,
      status: 'completed',
    },
  });
  if (resultNode?.error) {
    throw new Error(`Failed to create error tool-result node under "${resultParent}": ${resultNode.error}`);
  }
}
