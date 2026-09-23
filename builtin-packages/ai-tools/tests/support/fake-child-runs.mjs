/**
 * A fake of core's child-run API for the delegation tests, with the shapes the
 * function bindings answer (`raisin.agentRuns.spawnChild / children /
 * inspectChild / controlChild / mailbox / ackMailbox`): a parent run whose
 * active operation a test sets, children keyed by spawn key, hand-backs that
 * land in the parent's mailbox, and controls recorded per child.
 */
import { fakeRaisin } from './fake-raisin.mjs';

export const PARENT = 'run-parent0001';
export const CHAT = '/agents/lead/inbox/chats/c1';

const TERMINAL = new Set(['completed', 'failed', 'stopped']);

export function childWorld({ leadProps = {}, depth = 0 } = {}) {
  const runs = {};
  const f = fakeRaisin({ runs });
  const spawns = [];
  const childControls = [];
  let acked = 0;
  runs[PARENT] = {
    status: 'running',
    run: {
      run_id: PARENT, subject: { workspace: 'ai', path: CHAT }, agent_ref: 'functions:/agents/lead', depth,
      principal: { kind: 'agent', id: 'functions:/agents/lead', on_behalf_of: 'user-1' },
      state: { status: 'running', activity: { activity: 'operating', op: { op_id: `${PARENT}/op/1`, input: { tool: '/x' } } } },
      children: [], mailbox: [], usage: {},
    },
    projection: { items: [{ key: 't1', title: 'Review', status: 'in_progress' }] },
  };
  const parent = () => runs[PARENT].run;
  const link = (id) => parent().children.find((l) => l.run_id === id);

  Object.assign(f.api.agentRuns, {
    async spawnChild(req) {
      spawns.push(req);
      if (req.run_id !== PARENT) throw new Error('not_found: run not found');
      const found = parent().children.find((l) => l.spawn_key && l.spawn_key === req.spawn_key);
      if (found) return { child_run_id: found.run_id, child_no: found.child_no, created: false, budgets: req.budgets, resume_key: `child:${found.run_id}` };
      const child_no = parent().children.length + 1;
      const run_id = `run-child${String(child_no).padStart(4, '0')}`;
      parent().children.push({ child_no, run_id, title: req.objective.title, spawn_key: req.spawn_key, status: 'queued', delivered: false });
      runs[run_id] = {
        status: 'queued',
        run: {
          run_id, subject: req.subject, agent_ref: req.agent_ref, parent_run_id: PARENT, depth: depth + 1,
          principal: { kind: 'agent', id: req.as_agent, on_behalf_of: 'user-1' },
          delegation: { objective: req.objective, child_no, spawn_key: req.spawn_key },
          executor_config: req.executor_config, state: { status: 'queued', open: [] }, usage: {},
        },
      };
      return { child_run_id: run_id, child_no, created: true, budgets: req.budgets, resume_key: `child:${run_id}` };
    },
    async children({ run_id }) {
      if (run_id !== PARENT) throw new Error('not_found');
      return parent().children.map((l) => ({ link: { ...l }, status: runs[l.run_id].status, usage: {} }));
    },
    async inspectChild({ run_id, child_run_id }) {
      if (run_id !== PARENT || !link(child_run_id)) throw new Error('unknown_child');
      const c = runs[child_run_id];
      return { run: { run: c.run, status: c.status }, events: [{ seq: 1, kind: { type: 'run_created' } }], usage: {}, checkpoint: null };
    },
    async controlChild(req) {
      if (!link(req.child_run_id)) throw new Error('unknown_child');
      childControls.push(req);
      if (req.action === 'interrupt' && req.mode !== 'pause') runs[req.child_run_id].status = 'stopped';
      return { ack: 'applied', seq: 20 + childControls.length };
    },
    async mailbox({ run_id }) {
      return run_id === PARENT ? parent().mailbox.filter((m) => m.item.mail_no > acked) : [];
    },
    // Synchronous, like the runtime's binding (a `.catch` on it throws).
    ackMailbox({ up_to }) {
      acked = Math.max(acked, up_to);
      return parent().mailbox.filter((m) => m.item.mail_no > acked).length;
    },
  });

  /** Make `op` of `tool` the parent's active operation; returns the tool context. */
  const at = (op, tool) => {
    parent().state = { status: 'running', activity: { activity: 'operating', op: { op_id: `${PARENT}/op/${op}`, input: { tool } } } };
    return { run_id: PARENT, operation_id: `${PARENT}/op/${op}`, workspace: 'ai', chat_path: CHAT };
  };
  /** A child reaches a terminal state and its hand-back lands in the parent (core's job). */
  const finish = (runId, kind = 'succeeded', message = 'done', artifacts = []) => {
    const c = runs[runId];
    c.status = kind === 'failed' ? 'failed' : 'completed';
    c.run.state = { status: 'terminal', terminal: c.status, outcome: { kind, message, detail: { artifacts } } };
    const l = link(runId);
    l.delivered = true;
    l.status = c.status;
    parent().mailbox.push({ item: { mail_no: parent().mailbox.length + 1, kind: 'completion', from_run: runId }, payload: { status: c.status } });
  };
  const post = (runId, message) => {
    parent().mailbox.push({ item: { mail_no: parent().mailbox.length + 1, kind: 'message', from_run: runId }, payload: message });
  };
  const running = (runId) => { runs[runId].status = 'running'; };

  f.put('ai', CHAT, { node_type: 'raisin:Conversation', properties: { agent_ref: '/agents/lead' } });
  f.put('ai', `${CHAT}/m1`, { node_type: 'raisin:Message', properties: { role: 'user', content: 'Review the docs and fix typos' } });
  f.put('functions', '/agents/lead', { node_type: 'raisin:AIAgent', properties: { title: 'Lead', tools: ['/lib/raisin/ai/spawn-agent'], ...leadProps } });
  f.put('functions', '/agents/reviewer', { node_type: 'raisin:AIAgent', properties: { title: 'Reviewer', tools: ['/lib/demo/search', '/lib/demo/write', '/lib/raisin/ai/spawn-agent'] } });
  f.put('functions', '/lib/demo/search', { node_type: 'raisin:Function', name: 'search', properties: { name: 'search', description: 'Search', read_only: true, input_schema: { type: 'object', properties: { q: { type: 'string' } } } } });
  f.put('functions', '/lib/demo/write', { node_type: 'raisin:Function', name: 'write', properties: { name: 'write', description: 'Write', input_schema: { type: 'object', properties: { path: { type: 'string' } } } } });
  f.put('functions', '/lib/raisin/ai/spawn-agent', { node_type: 'raisin:Function', name: 'spawn-agent', properties: { name: 'spawn-agent', description: 'Spawn', input_schema: { type: 'object', properties: { objective: { type: 'object' } } } } });

  return { f, runs, spawns, childControls, at, finish, post, running, isTerminal: (id) => TERMINAL.has(runs[id].status) };
}
