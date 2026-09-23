/**
 * The envelope wrapper of the delegation tools, and the operations they share
 * (inspect, message, wait, interrupt) — each a thin handler over core's child
 * runs (delegation-core.js).
 *
 * Delegation exists only inside an agent run: a child is a child OF A RUN.
 */

import { buildEnvelope, errorEnvelope, runContextOf, locator, keyToken } from './tool-envelope.js';
import { DELEGATION_FUNCTIONS, waitSatisfied, childDone } from './delegation-spec.js';
import {
  AI_WS, requireParentRun, listChildren, pickChild, childOf, childSnapshot, snapshots, readMailbox, isLive,
  controlChild,
} from './delegation-core.js';

const str = (v) => (typeof v === 'string' ? v.trim() : '');

function fail(message, cls) {
  return Object.assign(new Error(message), { error_class: cls });
}

/**
 * Run a delegation tool body. The body returns
 * `{ payload, status?, writes?, refs?, next?, resumeKey? }`.
 */
export async function delegationTool(input, body) {
  const ctx = runContextOf(input);
  if (!ctx) {
    return { success: false, error: 'Delegation works only inside an agent run (a child is a child of a run).' };
  }
  try {
    const r = await body(input || {});
    return buildEnvelope({
      operationId: ctx.operation_id,
      status: r.status || 'succeeded',
      payload: r.payload,
      writes: r.writes || [],
      artifactRefs: r.refs || [],
      suggestedNextActions: r.next || [],
      resumeKey: r.resumeKey || null,
      retryPolicy: { retryable: false, max_attempts: 1, backoff_ms: 0, reason: 'idempotent_by_operation_id' },
    });
  } catch (err) {
    return errorEnvelope(ctx.operation_id, err);
  }
}

/** inspect: one child (with recent run events) or all of them, plus unread child messages. */
export async function inspect(input, fnPath = DELEGATION_FUNCTIONS.inspect) {
  const parent = await requireParentRun(input, fnPath);
  const views = await listChildren(parent);
  const ref = str(input.agent || input.child || input.run_id);
  const messages = await readMailbox(parent);
  if (!ref) {
    const children = await snapshots(parent, views);
    return { payload: { children, live: children.filter((c) => !childDone(c)).length, ...(messages.length ? { messages } : {}) } };
  }
  const child = await childSnapshot(parent, childOf(views, ref), { events: input.include_events !== false });
  return { payload: { child, ...(messages.length ? { messages } : {}) } };
}

/**
 * message / steer: new input for a live child. It is written to the child's
 * transcript first (so its next model turn reads it in order), then queued
 * through core, which applies it at the child's next safe boundary.
 */
export async function message(input, fnPath = DELEGATION_FUNCTIONS.message) {
  const parent = await requireParentRun(input, fnPath);
  const text = str(input.text || input.message);
  if (!text) throw fail('text is required', 'invalid_input');
  const mode = str(input.mode) === 'steer' ? 'steer' : 'message';
  const view = childOf(await listChildren(parent), str(input.agent || input.child || input.run_id));
  if (!isLive(view)) throw fail('That child run has finished; start a new one instead.', 'conflict');
  const child = await childSnapshot(parent, view);
  const token = keyToken(parent.opId);
  const content = mode === 'steer' ? `[Redirect from the delegating agent] ${text}` : `[Message from the delegating agent] ${text}`;
  const writes = [];
  let msgPath = null;
  if (child.chat_path) {
    const name = `parent-${token}`;
    msgPath = `${child.chat_path}/${name}`;
    if (!(await raisin.nodes.get(AI_WS, msgPath))) {
      await raisin.nodes.create(AI_WS, child.chat_path, {
        name,
        node_type: 'raisin:Message',
        properties: {
          role: 'user', content, body: { content, message_text: content }, status: 'delivered',
          message_type: `delegation_${mode}`, sender_id: `run:${parent.runId}`, agent_run_id: view.link.run_id, run_steer_state: 'queued',
        },
      });
      writes.push({ locator: locator(AI_WS, msgPath), action: 'created' });
    }
  }
  const body = { text: content, ...(msgPath ? { message_path: msgPath } : {}) };
  const ack = await controlChild(parent, view.link.run_id, `${mode}:${token}`,
    mode === 'steer' ? { action: 'steer', input: body } : { action: 'message', message: body });
  if (!ack || ack.ack === 'rejected') throw fail(`The child did not take the ${mode}: ${ack && ack.reason}`, 'conflict');
  if (view.status === 'paused') {
    await controlChild(parent, view.link.run_id, `resume:${token}`, { action: 'resume' }).catch(() => null);
  }
  return {
    payload: { child: view.link.run_id, mode, state: 'queued', ack, message: `The ${mode} is queued; the child reads it at its next safe boundary.` },
    writes,
  };
}

/** interrupt: stop (default) or pause one child, or every live child with `all: true`. */
export async function interrupt(input, fnPath = DELEGATION_FUNCTIONS.interrupt) {
  const parent = await requireParentRun(input, fnPath);
  const mode = str(input.mode) === 'pause' ? 'pause' : 'stop';
  const reason = str(input.reason) || 'Interrupted by the delegating agent';
  const views = await listChildren(parent);
  const targets = input.all === true || str(input.agent) === '*'
    ? views
    : [childOf(views, str(input.agent || input.child || input.run_id))];
  const token = keyToken(parent.opId);
  const children = [];
  for (const v of targets) {
    if (!isLive(v)) {
      children.push({ run_id: v.link.run_id, key: v.link.spawn_key, ack: 'already_finished' });
      continue;
    }
    const ack = await controlChild(parent, v.link.run_id, `interrupt-${mode}:${token}:${v.link.child_no}`, { action: 'interrupt', mode, reason });
    children.push({ run_id: v.link.run_id, key: v.link.spawn_key, ack: ack && ack.ack, reason: ack && ack.reason });
  }
  return {
    payload: { mode, children, interrupted: children.filter((c) => c.ack === 'applied' || c.ack === 'duplicate').length },
  };
}

/**
 * wait: answer at once when the waited children are done (mode `all`) or one
 * is (mode `any`); otherwise answer `waiting` on the next live child's resume
 * key (`child:{id}`). Core then delivers that child's hand-back into this very
 * operation — even one that landed before the wait was committed — and the
 * reducer calls the wait again for the rest, so the model gets one answer.
 */
export async function wait(input, fnPath = DELEGATION_FUNCTIONS.wait) {
  const parent = await requireParentRun(input, fnPath);
  const mode = str(input.mode) === 'any' ? 'any' : 'all';
  const views = await listChildren(parent);
  const refs = Array.isArray(input.agents) ? input.agents.map(str).filter(Boolean) : [];
  const chosen = refs.length ? refs.map((r) => childOf(views, r)) : views;
  if (!chosen.length) throw fail('This run has no child runs to wait for.', 'invalid_input');
  const children = await snapshots(parent, chosen);
  if (waitSatisfied(mode, children)) {
    const messages = await readMailbox(parent);
    const done = children.filter(childDone);
    return {
      payload: {
        done: true,
        mode,
        children,
        finished: done.length,
        pending: children.filter((c) => !childDone(c)).map((c) => c.key || c.run_id),
        ...(messages.length ? { messages } : {}),
        message: `${done.length}/${children.length} child run(s) finished.`,
      },
    };
  }
  const next = chosen.find((v) => !childDone(children.find((c) => c.run_id === v.link.run_id)));
  return {
    status: 'waiting',
    resumeKey: `child:${next.link.run_id}`,
    payload: {
      waiting_for: children.filter((c) => !childDone(c)).map((c) => c.key || c.run_id),
      mode,
      message: 'Waiting for the child runs; each hand-back arrives through the run mailbox.',
    },
  };
}

export { pickChild };
