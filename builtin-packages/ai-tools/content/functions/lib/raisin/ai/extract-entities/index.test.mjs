import assert from 'node:assert/strict';
import test from 'node:test';

import { handler } from './index.js';

const DOC_PATH = '/docs/msa';

/**
 * Stubs the node store, the model and the SQL surface.
 *
 * `nodes` is a { 'workspace:path': node } map that writes land in, so a second
 * run of the handler sees what the first one wrote — which is what makes the
 * idempotency test meaningful rather than a mock asserting itself.
 */
function stubRaisin({ graph, nodes = {}, model = 'openai:gpt-4o' }) {
  const calls = { statements: [], completions: 0 };

  globalThis.raisin = {
    nodes: {
      async get(ws, path) {
        return nodes[`${ws}:${path}`] || null;
      },
      async create(ws, parent, spec) {
        const path = `${parent === '/' ? '' : parent}/${spec.name}`;
        nodes[`${ws}:${path}`] = {
          id: `id${Object.keys(nodes).length}`,
          path,
          name: spec.name,
          node_type: spec.node_type,
          properties: { ...spec.properties },
        };
        return nodes[`${ws}:${path}`];
      },
      async update(ws, path, patch) {
        const existing = nodes[`${ws}:${path}`];
        if (!existing) throw new Error(`no node ${path}`);
        existing.properties = { ...existing.properties, ...patch.properties };
        return existing;
      },
    },
    ai: {
      getDefaultModel: () => model,
      async completion() {
        calls.completions += 1;
        return { content: JSON.stringify(graph), model };
      },
    },
    sql: {
      async execute(sql, params) {
        calls.statements.push({ sql, params });
        return { rows_affected: 1 };
      },
      async query() {
        return [];
      },
    },
  };

  return { calls, nodes };
}

function docNode(text) {
  return {
    id: 'doc-1',
    path: DOC_PATH,
    name: 'msa',
    node_type: 'raisin:Asset',
    properties: { title: 'MSA', __extracted_text: text },
  };
}

const TEXT = 'Dana Weber works at Acme Ltd in Zurich. The agreement runs for three years.';

const GRAPH = {
  entities: [
    { name: 'Dana Weber', type: 'person' },
    { name: 'Acme Ltd', type: 'organization' },
  ],
  relations: [{ from: 'Dana Weber', to: 'Acme Ltd', type: 'works at' }],
};

test('entities become nodes and edges become RELATE statements', async () => {
  const { calls } = stubRaisin({
    graph: GRAPH,
    nodes: { [`content:${DOC_PATH}`]: docNode(TEXT) },
  });

  const out = await handler({ path: DOC_PATH });

  assert.equal(out.count, 2);
  assert.deepEqual(
    out.entities.map((e) => e.path),
    ['/entities/dana-weber', '/entities/acme-ltd']
  );

  const relates = calls.statements.filter((s) => s.sql.includes('RELATE'));
  assert.equal(
    relates.length,
    3,
    'two mentioned_in edges plus one works_at — edges are RELATE rows, not ' +
      'reference properties, so they carry a type and live in the relation index'
  );
  assert.ok(relates.some((s) => s.sql.includes("TYPE 'works_at'")));
  assert.ok(relates.some((s) => s.sql.includes("TYPE 'mentioned_in'")));
});

test('paths are bound; only the sanitized edge type is interpolated', async () => {
  const { calls } = stubRaisin({
    graph: {
      entities: [{ name: "O'Brien", type: 'person' }, { name: 'Acme', type: 'organization' }],
      relations: [{ from: "O'Brien", to: 'Acme', type: "x' OR 1=1 --" }],
    },
    nodes: { [`content:${DOC_PATH}`]: docNode(TEXT) },
  });

  await handler({ path: DOC_PATH });

  for (const stmt of calls.statements) {
    assert.ok(
      !stmt.sql.includes("OR 1=1"),
      'a model-supplied relation type reaches a SQL literal position; it must ' +
        'be reduced to [a-z0-9_] rather than passed through'
    );
    assert.match(stmt.sql, /TYPE '[a-z0-9_]+'/);
  }
});

test('an unchanged document is not re-extracted', async () => {
  const shared = { [`content:${DOC_PATH}`]: docNode(TEXT) };

  const first = stubRaisin({ graph: GRAPH, nodes: shared });
  await handler({ path: DOC_PATH });
  assert.equal(first.calls.completions, 1);

  // Same node store, so the run marker written above is visible.
  const second = stubRaisin({ graph: GRAPH, nodes: shared });
  const out = await handler({ path: DOC_PATH });

  assert.equal(out.skipped, true);
  assert.equal(
    second.calls.completions,
    0,
    'a steady-state run must cost nothing; otherwise every re-save pays for a ' +
      'model call and mints a revision, which triggers another reindex'
  );
});

test('changed text re-extracts', async () => {
  const shared = { [`content:${DOC_PATH}`]: docNode(TEXT) };
  stubRaisin({ graph: GRAPH, nodes: shared });
  await handler({ path: DOC_PATH });

  shared[`content:${DOC_PATH}`] = docNode(`${TEXT} A new clause was added.`);
  const second = stubRaisin({ graph: GRAPH, nodes: shared });
  const out = await handler({ path: DOC_PATH });

  assert.equal(out.skipped, false);
  assert.equal(second.calls.completions, 1);
});

test('`force` re-extracts an unchanged document', async () => {
  const shared = { [`content:${DOC_PATH}`]: docNode(TEXT) };
  stubRaisin({ graph: GRAPH, nodes: shared });
  await handler({ path: DOC_PATH });

  const second = stubRaisin({ graph: GRAPH, nodes: shared });
  const out = await handler({ path: DOC_PATH, force: true });

  assert.equal(out.skipped, false);
  assert.equal(second.calls.completions, 1);
});

test('two documents naming the same entity converge on one node', async () => {
  const shared = {
    [`content:${DOC_PATH}`]: docNode(TEXT),
    'content:/docs/nda': {
      id: 'doc-2',
      path: '/docs/nda',
      name: 'nda',
      node_type: 'raisin:Asset',
      properties: { title: 'NDA', __extracted_text: 'Acme Ltd signed an NDA.' },
    },
  };

  stubRaisin({ graph: GRAPH, nodes: shared });
  await handler({ path: DOC_PATH });

  const second = stubRaisin({
    graph: { entities: [{ name: 'Acme Ltd', type: 'organization' }], relations: [] },
    nodes: shared,
  });
  const out = await handler({ path: '/docs/nda' });

  assert.equal(out.entities[0].created, false, 'the entity already existed');
  assert.equal(out.entities[0].path, '/entities/acme-ltd');

  const mentions = second.calls.statements.filter((s) =>
    s.sql.includes("TYPE 'mentioned_in'")
  );
  assert.equal(mentions.length, 1);
  assert.ok(
    mentions[0].params.includes('/docs/nda'),
    'the second document must attach to the SAME entity node — that shared ' +
      'node is the connection a vector search over one document cannot make'
  );
});

test('a relation naming an entity we did not write is dropped', async () => {
  const { calls } = stubRaisin({
    graph: {
      entities: [{ name: 'Acme', type: 'organization' }],
      relations: [{ from: 'Acme', to: 'Someone Not Extracted', type: 'employs' }],
    },
    nodes: { [`content:${DOC_PATH}`]: docNode(TEXT) },
  });

  const out = await handler({ path: DOC_PATH });

  assert.deepEqual(out.relations, [], 'a dangling edge traverses to nothing and reports no fault');
  assert.ok(!calls.statements.some((s) => s.sql.includes("TYPE 'employs'")));
});

test('an unparseable model answer yields no entities instead of throwing', async () => {
  globalThis.raisin = undefined;
  const { calls } = stubRaisin({
    graph: GRAPH,
    nodes: { [`content:${DOC_PATH}`]: docNode(TEXT) },
  });
  raisin.ai.completion = async () => {
    calls.completions += 1;
    return { content: 'Sorry, I cannot help with that.' };
  };

  const out = await handler({ path: DOC_PATH });

  assert.equal(out.count, 0);
  assert.equal(out.skipped, false);
});

test('a document with no text is skipped before the model is called', async () => {
  const { calls } = stubRaisin({
    graph: GRAPH,
    nodes: {
      [`content:${DOC_PATH}`]: {
        id: 'doc-1',
        path: DOC_PATH,
        name: 'msa',
        properties: { title: 'MSA' },
      },
    },
  });

  const out = await handler({ path: DOC_PATH });

  assert.equal(out.skipped, true);
  assert.equal(calls.completions, 0);
});
