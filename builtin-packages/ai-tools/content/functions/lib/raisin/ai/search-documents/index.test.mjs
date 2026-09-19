import assert from 'node:assert/strict';
import test from 'node:test';

import { handler } from './index.js';

/** Captures the SQL and params the handler issues, and answers with `rows`. */
function stubRaisin(rows) {
  const captured = {};
  globalThis.raisin = {
    sql: {
      async query(sql, params) {
        captured.sql = sql;
        captured.params = params;
        return rows;
      },
    },
  };
  return captured;
}

test('asks for PASSAGES, not documents', async () => {
  const captured = stubRaisin([]);

  await handler({ query: 'notice period' });

  assert.match(
    captured.sql,
    /granularity\s*=>\s*'chunk'/,
    'without chunk granularity a limit counts whole documents, which do not ' +
      'fit in a context window'
  );
});

test('caller-controlled values are BOUND, never interpolated', async () => {
  const captured = stubRaisin([]);

  await handler({ query: "o'brien", workspaces: 'docs, handbook' });

  assert.deepEqual(captured.params, ["o'brien", 'docs, handbook']);
  assert.ok(
    !captured.sql.includes("o'brien"),
    'the query text must not be concatenated into the SQL'
  );
});

test('the limit is clamped, because each leg over-fetches 20x', async () => {
  const captured = stubRaisin([]);

  await handler({ query: 'x', limit: 5000 });

  assert.match(captured.sql, /HYBRID_SEARCH\(\$1, 50,/);
});

test('defaults to every readable workspace, spelled the one accepted way', async () => {
  const captured = stubRaisin([]);

  await handler({ query: 'x' });

  assert.equal(captured.params[1], 'ALL READABLE');
});

test('a preview passage is reported as not exact', async () => {
  stubRaisin([
    {
      node_id: 'n1',
      path: '/contracts/msa',
      name: 'MSA',
      score: 0.9,
      chunk_index: 3,
      chunk_text: 'Either party may terminate…',
      chunk_text_source: 'exact',
    },
    {
      node_id: 'n2',
      path: '/contracts/nda',
      name: 'NDA',
      score: 0.4,
      chunk_index: 0,
      chunk_text: 'truncated preview…',
      chunk_text_source: 'excerpt',
    },
  ]);

  const { results, count } = await handler({ query: 'terminate' });

  assert.equal(count, 2);
  assert.equal(results[0].text_is_exact, true);
  assert.equal(
    results[1].text_is_exact,
    false,
    'an excerpt is a 200-character preview; a caller that cannot tell will ' +
      'quote it as though it were the whole clause'
  );
});

test('chunk_index 0 survives — it is a real chunk, not a missing one', async () => {
  stubRaisin([
    {
      node_id: 'n1',
      path: '/notes/one',
      name: 'One',
      score: 0.5,
      chunk_index: 0,
      chunk_text: 'body',
      chunk_text_source: 'exact',
    },
  ]);

  const { results } = await handler({ query: 'x' });

  assert.equal(results[0].chunk_index, 0);
});

test('a passage with no text is dropped rather than cited as empty', async () => {
  stubRaisin([
    {
      node_id: 'n1',
      path: '/a',
      name: 'A',
      score: 0.5,
      chunk_index: 1,
      chunk_text: null,
      chunk_text_source: 'unavailable',
    },
  ]);

  const { results, count } = await handler({ query: 'x' });

  assert.equal(count, 0);
  assert.deepEqual(results, []);
});

test('an empty query is rejected', async () => {
  stubRaisin([]);

  await assert.rejects(() => handler({ query: '   ' }), /non-empty/);
});
