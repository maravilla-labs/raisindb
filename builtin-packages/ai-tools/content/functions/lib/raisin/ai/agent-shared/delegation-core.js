/**
 * Delegation over RaisinDB core's CHILD RUNS — the half that talks to the
 * runtime.
 *
 * Core owns everything durable about a child: the lineage and its admission
 * (`spawnChild`, idempotent by spawn key), the hand-back into the parent's
 * mailbox, the answer to a tool that waits on `child:{id}`, the cascade stop
 * of live children when the parent ends, and the repair of all of it after a
 * crash (jobs any node picks up, the sweeper). Nothing here stores delegation
 * state: a child is read from core (`children`, `inspectChild`), controlled
 * through core (`controlChild`), and its conversation is only its transcript.
 *
 * The delegation tools run in a SYSTEM context (they write the child's
 * transcript into another agent's inbox), so each one first proves it is the
 * calling run's active operation (`requireRunOperation`).
 */

import { requireRunOperation } from './run-caller.js';
import {
  TERMINAL_STATUSES, childDone, acceptance, acceptanceReads, specOfObjective, agentPath, taskKey,
} from './delegation-spec.js';

export const AI_WS = 'ai';

const str = (v) => (typeof v === 'string' ? v.trim() : '');

/** The calling run, proven (see run-caller.js). */
export function requireParentRun(input, functionPath) {
  return requireRunOperation(input, functionPath, { what: 'Delegation' });
}

/** Create every missing folder on `path`. */
export async function ensureFolders(workspace, path) {
  const parts = String(path).split('/').filter(Boolean);
  let cur = '';
  for (const part of parts) {
    const next = `${cur}/${part}`;
    if (!(await raisin.nodes.get(workspace, next))) {
      try {
        await raisin.nodes.create(workspace, cur || '/', { name: part, node_type: 'raisin:Folder', properties: { title: part } });
      } catch (err) {
        if (!/already exists/i.test(String(err && err.message))) throw err;
      }
    }
    cur = next;
  }
}

/** Every child of the parent run, as core lists them (spawn order). */
export async function listChildren(parent) {
  const views = await raisin.agentRuns.children({ run_id: parent.runId });
  return Array.isArray(views) ? views : [];
}

/** A child by run id, key, task id, number or title. */
export function pickChild(views, ref) {
  const want = str(ref);
  if (!want) return null;
  return views.find((v) => {
    const l = (v && v.link) || {};
    return l.run_id === want || l.spawn_key === want || l.spawn_key === taskKey(want)
      || String(l.child_no) === want || l.title === want;
  }) || null;
}

/** `pickChild` or a `not_found` error naming the children that exist. */
export function childOf(views, ref) {
  const v = pickChild(views, ref);
  if (v) return v;
  const known = views.map((c) => c.link.spawn_key || c.link.run_id).join(', ') || 'none';
  throw Object.assign(new Error(`No child run "${ref}" belongs to this run (children: ${known}).`), { error_class: 'not_found' });
}

/**
 * One child as the parent reads it: status, hand-back, outcome, artifacts,
 * usage, and — once it is done — acceptance evaluated on STORED state.
 */
export async function childSnapshot(parent, view, { events = false } = {}) {
  const link = view.link || {};
  const insp = await raisin.agentRuns.inspectChild({
    run_id: parent.runId, child_run_id: link.run_id, after_seq: 0, limit: events ? 200 : 1,
  });
  const rec = (insp && insp.run && insp.run.run) || {};
  const status = (insp && insp.run && insp.run.status) || view.status || link.status || 'unknown';
  const outcome = (rec.state && rec.state.outcome) || null;
  const objective = rec.delegation && rec.delegation.objective;
  const detail = (outcome && outcome.detail) || {};
  const snap = {
    run_id: link.run_id,
    key: link.spawn_key || null,
    child_no: link.child_no,
    title: link.title || (objective && objective.title) || null,
    agent_ref: agentPath(rec.agent_ref) || rec.agent_ref || null,
    chat_path: (rec.subject && rec.subject.path) || null,
    status,
    handed_back: !!link.delivered,
    outcome: outcome ? outcome.kind : null,
    summary: outcome ? outcome.message || null : null,
    artifacts: (Array.isArray(detail.artifacts) ? detail.artifacts : []).slice(0, 20),
    usage: view.usage || (insp && insp.usage) || null,
  };
  if (outcome && outcome.code) snap.failure_code = outcome.code;
  if (events && insp && Array.isArray(insp.events)) {
    snap.recent_events = insp.events.slice(-15).map((e) => ({ seq: e.seq, type: e.kind && e.kind.type }));
  }
  if (childDone(snap)) {
    const spec = specOfObjective(objective);
    if (spec.checks.length || spec.expected_artifacts.length) {
      const nodes = new Map();
      for (const loc of acceptanceReads(spec)) {
        try {
          const n = await raisin.nodes.get(loc.workspace, loc.path);
          if (n) nodes.set(`${loc.workspace}:${loc.path}`, n);
        } catch (_) { /* unreadable = absent */ }
      }
      const written = snap.artifacts.map((a) => (a && a.locator) || a).filter(Boolean);
      const acc = acceptance(spec, { outcome: snap.outcome, written, nodes });
      snap.expected_artifacts = acc.expected;
      snap.checks = acc.checks;
      snap.accepted = acc.accepted;
    } else {
      snap.accepted = snap.outcome === 'succeeded';
    }
  }
  return snap;
}

/** Snapshots of many children. */
export async function snapshots(parent, views, opts) {
  const out = [];
  for (const v of views) out.push(await childSnapshot(parent, v, opts));
  return out;
}

/**
 * The parent's unread mailbox: messages its children posted (their
 * completions are reported through the snapshots). Reading acknowledges them,
 * so core's bounded mailbox never fills.
 */
export async function readMailbox(parent) {
  let items;
  try {
    items = await raisin.agentRuns.mailbox({ run_id: parent.runId });
  } catch (_) {
    return [];
  }
  const list = Array.isArray(items) ? items : [];
  if (!list.length) return [];
  const upTo = Math.max(...list.map((m) => Number(m.item && m.item.mail_no) || 0));
  const messages = list
    .filter((m) => m.item && m.item.kind === 'message')
    .map((m) => ({ from_run: m.item.from_run, mail_no: m.item.mail_no, message: m.payload }));
  // The runtime's bindings answer SYNCHRONOUSLY: no `.catch` on their result.
  if (upTo > 0) {
    try { await raisin.agentRuns.ackMailbox({ run_id: parent.runId, up_to: upTo }); } catch (_) { /* read again next time */ }
  }
  return messages;
}

/** Whether a child is still working. */
export function isLive(view) {
  const l = (view && view.link) || {};
  return !l.delivered && !TERMINAL_STATUSES.has(view.status);
}

/** Send a parent → child control through core (message, steer, interrupt, resume). */
export async function controlChild(parent, childRunId, controlId, action) {
  return raisin.agentRuns.controlChild({
    run_id: parent.runId, child_run_id: childRunId, control_id: controlId, ...action,
  });
}
