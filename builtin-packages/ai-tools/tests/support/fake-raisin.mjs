/**
 * An in-memory `raisin` for the agent-run harness tests: nodes, a small SQL
 * matcher (CHILD_OF / DESCENDANT_OF, node_type, property equality, ORDER BY
 * created_at, LIMIT), events, scripted completions and a recording
 * `agentRuns` binding.
 */

let clock = Date.parse('2026-09-23T10:00:00Z');
const tick = () => new Date(clock += 1000).toISOString();

export function fakeRaisin({ completions = [], runs = {} } = {}) {
  const nodes = new Map(); // `${ws}\0${path}` → node
  const events = [];
  const calls = { completions: [], controls: [], creates: [], outbox: [] };
  const key = (ws, path) => `${ws}\u0000${path}`;
  let ids = 0;

  const put = (ws, path, node) => {
    const n = {
      id: node.id || `id-${++ids}`, path, name: path.split('/').pop(), workspace: ws,
      created_at: node.created_at || tick(), ...node, properties: { ...(node.properties || {}) },
    };
    nodes.set(key(ws, path), n);
    return n;
  };
  const parentOf = (p) => p.slice(0, p.lastIndexOf('/')) || '/';

  const nodesApi = {
    async get(ws, path) { return nodes.get(key(ws, path)) || null; },
    async create(ws, parent, body) {
      const path = `${parent === '/' ? '' : parent}/${body.name}`;
      if (nodes.has(key(ws, path))) throw new Error(`Node already exists: ${path}`);
      if (parent.includes('/outbox')) calls.outbox.push(body);
      return put(ws, path, body);
    },
    async update(ws, path, body) {
      const n = nodes.get(key(ws, path));
      if (!n) throw new Error(`not found: ${path}`);
      n.properties = { ...n.properties, ...((body && body.properties) || {}) };
      return n;
    },
    // Synchronous, like the runtime's binding (a `.catch` on it throws).
    updateProperty(ws, path, k, v) {
      const n = nodes.get(key(ws, path));
      if (n) n.properties = { ...n.properties, [k]: v };
      return true;
    },
    async getChildren(ws, parent) {
      return [...nodes.values()].filter((n) => n.workspace === ws && parentOf(n.path) === parent);
    },
    beginTransaction() {
      return { create: (...a) => nodesApi.create(...a), commit() {}, rollback() {} };
    },
  };

  function query(sql, params = []) {
    const ws = (/FROM\s+["']([^"']+)["']/.exec(sql) || [])[1];
    const val = (tok) => (tok.startsWith('$') ? params[Number(tok.slice(1)) - 1] : tok.slice(1, -1));
    let rows = [...nodes.values()].filter((n) => !ws || n.workspace === ws);
    const child = /CHILD_OF\((\$\d)\)/.exec(sql);
    const desc = /DESCENDANT_OF\((\$\d)\)/.exec(sql);
    if (child) rows = rows.filter((n) => parentOf(n.path) === val(child[1]));
    if (desc) rows = rows.filter((n) => n.path.startsWith(`${val(desc[1])}/`));
    const one = /node_type\s*=\s*'([^']+)'/.exec(sql);
    const many = /node_type\s+IN\s*\(([^)]*)\)/.exec(sql);
    if (one) rows = rows.filter((n) => n.node_type === one[1]);
    if (many) {
      const set = many[1].split(',').map((s) => s.trim().replace(/'/g, ''));
      rows = rows.filter((n) => set.includes(n.node_type));
    }
    for (const m of sql.matchAll(/properties->>'(\w+)'(?:::\w+)?\s*=\s*(\$\d|'[^']*')/g)) {
      rows = rows.filter((n) => String(n.properties[m[1]]) === String(val(m[2])));
    }
    const idm = /\bid\s*=\s*(\$\d)/.exec(sql);
    if (idm) rows = rows.filter((n) => n.id === val(idm[1]));
    if (/created_at\s*>\s*\$\d/.test(sql)) {
      const at = params[params.length - 1];
      rows = rows.filter((n) => n.created_at > at);
    }
    rows.sort((a, b) => (a.created_at < b.created_at ? -1 : 1));
    if (/ORDER BY created_at DESC/i.test(sql)) rows.reverse();
    const lim = /LIMIT\s+(\d+)/i.exec(sql);
    if (lim) rows = rows.slice(0, Number(lim[1]));
    if (/COUNT\(\*\)/i.test(sql)) return [{ count: rows.length }];
    return rows.map((n) => ({ ...n }));
  }

  const agentRuns = {
    async create(req) {
      calls.creates.push(req);
      const live = Object.values(runs).find((r) => r.run.subject.path === req.subject.path
        && !['completed', 'failed', 'stopped'].includes(r.status));
      if (live) return { run_id: live.run.run_id, created: false, status: live.status };
      const run_id = `run-${Object.keys(runs).length + 1}abcdef`;
      runs[run_id] = { status: 'queued', run: { run_id, subject: req.subject, create_key: req.create_key, state: { status: 'queued', open: [] }, budgets: req.budgets, usage: {} }, projection: null };
      return { run_id, created: true, status: 'queued' };
    },
    async get({ run_id }) {
      if (!runs[run_id]) throw new Error('not_found: run not found');
      return runs[run_id];
    },
    async control(req) {
      calls.controls.push(req);
      return runs[req.run_id] && runs[req.run_id].reject
        ? { ack: 'rejected', reason: runs[req.run_id].reject, seq: 9 }
        : { ack: 'applied', seq: 10 + calls.controls.length };
    },
    async events({ run_id, after_seq = 0 }) {
      return ((runs[run_id] && runs[run_id].events) || []).filter((e) => e.seq > after_seq);
    },
  };

  const api = {
    nodes: nodesApi,
    sql: { async query(sql, params) { return query(sql, params); }, async execute() { return 0; } },
    events: { async emit(type, payload) { events.push({ type, payload }); } },
    crypto: { async uuid() { return `uuid-${++ids}`; } },
    ai: {
      async completion(req) {
        calls.completions.push(req);
        const next = completions.shift();
        if (next instanceof Error) throw next;
        if (typeof next === 'function') return next(req);
        return next || { content: 'ok', finish_reason: 'stop', model: 'm', tool_calls: [] };
      },
    },
    agentRuns,
  };
  globalThis.raisin = api;
  return { api, nodes, put, events, calls, runs };
}
