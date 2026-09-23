/**
 * agent-run-project — the run's PROJECTION operation.
 *
 * The generic reducer issues it (never a model): to write the plan card when
 * the plan in run state changed, to show a pending approval, and — once, at
 * the end — to deliver the final answer with the runtime's own status (a
 * delegated run's hand-back to its parent is core's, from the run's terminal
 * outcome, not this projection's). It
 * writes only the transcript (plan card, final message fields, outbox, UI
 * events), every write idempotent, and answers with a `raisin.tool-result/1`
 * envelope whose primary artifact is the card or the final message.
 *
 * Runs in a system context (it writes into the agent's inbox), so it first
 * checks that the run it claims to act for exists and is about this
 * conversation.
 */

import { log } from '../agent-shared/logger.js';
import { emitConversationEvent } from '../agent-shared/streaming.js';
import { resolveAgentOutboxContext, sendAgentOutboxMessage } from '../agent-shared/outbox.js';
import { readAssistantContent, updateAssistantContent, TERMINAL_FALLBACK_TEXT } from '../agent-shared/utils.js';
import { loadRunChat, findPreviousTurn } from '../agent-shared/run-transcript.js';
import { upsertPlanCard, syncPlanCards } from '../agent-shared/run-plan-card.js';
import { buildEnvelope, errorEnvelope, locator, runContextOf } from '../agent-shared/tool-envelope.js';

const nowIso = () => new Date().toISOString();

/** The run must exist and be about this conversation. */
async function verifyRun(ctx) {
  if (!raisin.agentRuns || typeof raisin.agentRuns.get !== 'function') return;
  const view = await raisin.agentRuns.get({ run_id: ctx.runId });
  const subject = view && view.run && view.run.subject;
  if (!subject || subject.path !== ctx.chatPath || (subject.workspace || 'ai') !== ctx.workspace) {
    throw new Error('agent-run-project: the run is not about this conversation');
  }
  return view;
}

function finalContent(lastText, finish) {
  const base = lastText && lastText.trim() ? lastText.trim() : TERMINAL_FALLBACK_TEXT;
  if (!finish || finish.outcome === 'succeeded' || !finish.status_statement) return base;
  return `${base}\n\nStatus (from the runtime): ${finish.status_statement}`;
}

async function finalize(ctx, args, outboxCtx) {
  const finish = args.finish || { outcome: 'succeeded' };
  const last = await findPreviousTurn(ctx.workspace, ctx.chatPath, ctx.runId, null);
  const content = finalContent(args.last_text, finish);
  const writes = [];
  if (last) {
    const node = await raisin.nodes.get(ctx.workspace, last.path);
    if (node && readAssistantContent(node.properties || {}) !== content) {
      await updateAssistantContent(ctx.workspace, node, content);
    }
    await raisin.nodes.updateProperty(ctx.workspace, last.path, 'run_outcome', finish.outcome);
    if (finish.status_statement) {
      await raisin.nodes.updateProperty(ctx.workspace, last.path, 'run_status_statement', finish.status_statement);
    }
    writes.push({ locator: locator(ctx.workspace, last.path), action: 'updated' });
  }
  if (outboxCtx) {
    await sendAgentOutboxMessage(ctx.workspace, outboxCtx, content, 'chat', {
      finish_reason: finish.failed ? 'error' : 'stop',
      run_id: ctx.runId,
      run_outcome: finish.outcome,
    }, { dedupe_key: `run_terminal:${ctx.runId}` });
  }
  await raisin.nodes.updateProperty(ctx.workspace, ctx.chatPath, 'last_agent_run_id', ctx.runId);
  await raisin.nodes.updateProperty(ctx.workspace, ctx.chatPath, 'last_agent_run_outcome', finish.outcome);
  await emitConversationEvent('conversation:done', {
    type: 'done',
    content,
    role: 'assistant',
    senderDisplayName: outboxCtx ? outboxCtx.agentDisplayName : null,
    finishReason: finish.failed ? 'error' : 'stop',
    dispatchPhase: 'terminal',
    runId: ctx.runId,
    runOutcome: finish.outcome,
    ...(finish.status_statement ? { serverStatus: finish.status_statement, verified: finish.outcome === 'succeeded' } : {}),
    timestamp: nowIso(),
  }, ctx.chatPath, ctx.streamChannel);
  return writes;
}

export async function handler(input = {}) {
  const run = runContextOf(input);
  const rc = (input && input.__raisin_context) || {};
  const opId = run ? run.operation_id : 'unknown';
  try {
    if (!run) throw new Error('agent-run-project runs only inside an agent run');
    if (!rc.chat_path) throw new Error('agent-run-project needs the conversation path');
    const base = await loadRunChat({ workspace: rc.workspace || 'ai', path: rc.chat_path });
    const ctx = { ...base, runId: run.run_id, operationId: opId };
    await verifyRun(ctx);
    const outboxCtx = await resolveAgentOutboxContext(ctx.workspace, ctx.chatPath, ctx.chat);

    const writes = [];
    const refs = [];
    let planPath = null;
    if (input.plan) {
      planPath = await upsertPlanCard(ctx, input.plan, rc.msg_path || null);
      if (planPath) {
        const data = await syncPlanCards(ctx, outboxCtx, input.plan, planPath);
        writes.push({ locator: locator(ctx.workspace, planPath), action: 'updated' });
        refs.push({ kind: 'plan_card', logical_key: `plan-${input.plan.no}`, locator: locator(ctx.workspace, planPath), role: 'primary' });
        if (input.reason === 'approval') {
          await emitConversationEvent('conversation:waiting', {
            type: 'waiting',
            reason: 'awaiting_plan_approval',
            planPath,
            plan: data,
            runId: ctx.runId,
            timestamp: nowIso(),
          }, ctx.chatPath, ctx.streamChannel);
        }
      }
    }
    if (input.reason === 'final') {
      const finalWrites = await finalize(ctx, input, outboxCtx);
      writes.push(...finalWrites);
      if (!refs.length && finalWrites.length) {
        refs.push({ kind: 'message', locator: finalWrites[0].locator, role: 'primary' });
      }
    }
    return buildEnvelope({
      operationId: opId,
      status: 'succeeded',
      payload: { reason: input.reason || 'plan', plan_path: planPath },
      writes,
      artifactRefs: refs,
    });
  } catch (err) {
    log.error('agent-run-project', 'Projection failed', { error: String(err && err.message) });
    return errorEnvelope(opId, err);
  }
}

