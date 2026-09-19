import assert from 'node:assert/strict';
import test from 'node:test';

import { handler } from './index.js';

/**
 * Stubs the relation index as an adjacency map: { '<ref>': [row, ...] }.
 * Rows use the columns NEIGHBORS actually emits — `id`, `path`, `name`,
 * `node_type`, `relation_type`, `weight`.
 */
function stubGraph(adjacency, { search = { results: [] } } = {}) {
  const calls = { queries: [] };
  globalThis.raisin = {
    functions: {
      // The function the code actually calls: `call`, not `execute` — the
      // latter is the AI-tool-call form and needs a tool-call node.
      async call() {
        return search;
      },
    },
    sql: {
      async query(sql, params) {
        calls.queries.push(params[0]);
        return adjacency[params[0]] || [];
      },
    },
  };
  return calls;
}

function node(path, relation_type = 'mentioned_in') {
  return {
    id: `id-${path}`,
    path,
    name: path.split('/').pop(),
    node_type: 'raisin:Folder',
    relation_type,
    weight: null,
  };
}

test('explicit seeds skip the search step entirely', async () => {
  const calls = stubGraph({ '/entities/acme': [node('/docs/msa')] });

  const out = await handler({ seeds: ['/entities/acme'] });

  assert.equal(out.count, 1);
  assert.equal(out.nodes[0].path, '/docs/msa');
  assert.deepEqual(calls.queries, ['/entities/acme']);
});

test('the edge TYPE is carried through as `via`', async () => {
  stubGraph({ '/entities/dana': [node('/entities/acme', 'works_at')] });

  const out = await handler({ seeds: ['/entities/dana'] });

  assert.equal(
    out.nodes[0].via,
    'works_at',
    'without the edge label an agent can say two things are connected but ' +
      'never how'
  );
});

test('a second hop is only walked when asked for', async () => {
  // Keyed by what the walk actually passes to NEIGHBORS: the seed's own
  // reference first, then each neighbour's node ID.
  const adjacency = {
    '/a': [node('/b')],
    'id-/b': [node('/c')],
  };

  stubGraph(adjacency);
  const oneHop = await handler({ seeds: ['/a'], hops: 1 });
  assert.deepEqual(oneHop.nodes.map((n) => n.path), ['/b']);

  stubGraph(adjacency);
  const twoHops = await handler({ seeds: ['/a'], hops: 2 });
  assert.deepEqual(twoHops.nodes.map((n) => n.path), ['/b', '/c']);
  assert.deepEqual(twoHops.nodes.map((n) => n.hop), [1, 2]);
});

test('a node reachable two ways is returned once, at the shorter distance', async () => {
  stubGraph({
    '/a': [node('/b'), node('/c')],
    'id-/b': [node('/c')],
    'id-/c': [],
  });

  const out = await handler({ seeds: ['/a'], hops: 2 });

  const cs = out.nodes.filter((n) => n.path === '/c');
  assert.equal(cs.length, 1, 'a diamond must not duplicate its far corner');
  assert.equal(cs[0].hop, 1, 'the shorter path must win, or `hop` misreports distance');
});

test('a seed is never returned as its own neighbour', async () => {
  stubGraph({ '/a': [node('/a'), node('/b')] });

  const out = await handler({ seeds: ['/a'] });

  assert.deepEqual(out.nodes.map((n) => n.path), ['/b']);
});

test('hitting the budget is reported, not silently swallowed', async () => {
  const hub = Array.from({ length: 25 }, (_, i) => node(`/n${i}`));
  const adjacency = { '/hub': hub };
  for (let i = 0; i < 25; i += 1) {
    adjacency[`id-/n${i}`] = Array.from({ length: 25 }, (_, j) => node(`/m${i}-${j}`));
  }
  stubGraph(adjacency);

  const out = await handler({ seeds: ['/hub'], hops: 3 });

  assert.equal(
    out.truncated,
    true,
    'a truncated neighbourhood that looks complete is how an agent concludes ' +
      'something is unconnected when it was simply not looked at'
  );
  assert.ok(out.count <= 60);
});

test('seeds come from search when none are given', async () => {
  const calls = stubGraph(
    { n1: [node('/entities/acme')] },
    {
      search: {
        results: [
          { path: '/docs/msa', node_id: 'n1', title: 'MSA', text: 'x' },
          { path: '/docs/msa', node_id: 'n1', title: 'MSA', text: 'y' },
        ],
      },
    }
  );

  const out = await handler({ query: 'acme contract' });

  assert.equal(out.seeds.length, 1, 'two passages of one document are one seed');
  assert.deepEqual(calls.queries, ['n1'], 'a search hit is traversed by its node id');
  assert.equal(out.nodes[0].path, '/entities/acme');
});

test('the walk continues by node ID, not by path', async () => {
  // A bare path resolves inside the DEFAULT workspace only, and a neighbour row
  // does not say which workspace it came from — so continuing by path searched
  // the wrong workspace and returned nothing, which reads as "the graph ends
  // here". This is that regression.
  const calls = stubGraph({
    '/start': [node('/second')],
    'id-/second': [node('/third')],
  });

  const out = await handler({ seeds: ['/start'], hops: 2 });

  assert.deepEqual(
    calls.queries,
    ['/start', 'id-/second'],
    'hop two must address the node by id'
  );
  assert.deepEqual(out.nodes.map((n) => n.path), ['/second', '/third']);
});

test('a failed seed search is an error, not an empty graph', async () => {
  stubGraph({}, { search: { error: 'workspace not readable' } });

  await assert.rejects(
    () => handler({ query: 'anything' }),
    /Seed search failed/,
    '"nothing is connected" reads as a fact about the graph; it must not be a ' +
      'swallowed error'
  );
});

test('neither a query nor seeds is rejected', async () => {
  stubGraph({});
  await assert.rejects(() => handler({}), /`query` or a non-empty `seeds`/);
});
