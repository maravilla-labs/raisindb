import { describe, expect, it, vi } from 'vitest';

import { SearchApi } from './search-api';
import type { SqlResult } from './protocol';

function sqlResult(rows: Record<string, unknown>[]): SqlResult {
  return { rows } as unknown as SqlResult;
}

/** A SearchApi whose SQL and function calls are recorded. */
function harness(options: { rows?: Record<string, unknown>[]; answer?: unknown } = {}) {
  const calls: { sql: string[]; params: unknown[][]; fn: [string, unknown][] } = {
    sql: [],
    params: [],
    fn: [],
  };

  const api = new SearchApi(
    async (sql, params) => {
      calls.sql.push(sql);
      calls.params.push(params);
      return sqlResult(options.rows ?? []);
    },
    async (name, input) => {
      calls.fn.push([name, input]);
      return options.answer ?? {};
    },
  );

  return { api, calls };
}

const PASSAGE_ROW = {
  node_id: 'n1',
  path: '/contracts/msa',
  name: 'MSA',
  score: 0.81,
  chunk_index: 3,
  chunk_text: 'Either party may terminate on thirty days written notice.',
  chunk_text_source: 'exact',
};

describe('search', () => {
  it('asks for passages, not documents', async () => {
    const { api, calls } = harness();

    await api.search('notice period', { workspaces: 'stories' });

    expect(calls.sql[0]).toMatch(/granularity\s*=>\s*'chunk'/);
  });

  it('binds the query and the scope instead of pasting them', async () => {
    const { api, calls } = harness();

    await api.search("o'brien", { workspaces: 'stories, handbook' });

    expect(calls.params[0]).toEqual(["o'brien", 'stories, handbook']);
    expect(calls.sql[0]).not.toContain("o'brien");
  });

  it('requires a scope, because an unscoped public search leaks drafts', async () => {
    const { api } = harness();

    await expect(
      api.search('anything', { workspaces: '  ' }),
    ).rejects.toThrow(/workspaces/);
  });

  it('clamps the limit', async () => {
    const { api, calls } = harness();

    await api.search('x', { workspaces: 'stories', limit: 10_000 });

    expect(calls.sql[0]).toMatch(/HYBRID_SEARCH\(\$1, 50,/);
  });

  it('maps a row to a passage, preserving chunk 0', async () => {
    const { api } = harness({ rows: [{ ...PASSAGE_ROW, chunk_index: 0 }] });

    const [passage] = await api.search('x', { workspaces: 'stories' });

    expect(passage).toMatchObject({
      path: '/contracts/msa',
      nodeId: 'n1',
      chunkIndex: 0,
      textIsExact: true,
    });
  });

  it('marks a preview passage as not exact', async () => {
    const { api } = harness({
      rows: [{ ...PASSAGE_ROW, chunk_text_source: 'excerpt' }],
    });

    const [passage] = await api.search('x', { workspaces: 'stories' });

    expect(passage.textIsExact).toBe(false);
  });

  it('drops a passage with no text rather than returning an empty citation', async () => {
    const { api } = harness({
      rows: [{ ...PASSAGE_ROW, chunk_text: null, chunk_text_source: 'unavailable' }],
    });

    expect(await api.search('x', { workspaces: 'stories' })).toEqual([]);
  });
});

describe('ask', () => {
  it('passes the options through under the names the function expects', async () => {
    const { api, calls } = harness({ answer: { answer: 'a', grounded: true } });

    await api.ask('How much notice?', {
      workspaces: 'stories',
      limit: 4,
      useGraph: true,
    });

    expect(calls.fn[0][0]).toBe('ask');
    expect(calls.fn[0][1]).toMatchObject({
      question: 'How much notice?',
      workspaces: 'stories',
      limit: 4,
      use_graph: true,
    });
  });

  it('maps the answer, its citations and its attempts', async () => {
    const { api } = harness({
      answer: {
        answer: 'Thirty days [1].',
        grounded: true,
        model: 'openai:gpt-4o',
        citations: [
          {
            marker: 1,
            path: '/contracts/msa',
            node_id: 'n1',
            title: 'MSA',
            chunk_index: 3,
            text_is_exact: true,
          },
        ],
        attempts: [{ query: 'How much notice?', passages: 8 }],
      },
    });

    const result = await api.ask('How much notice?', { workspaces: 'stories' });

    expect(result.answer).toBe('Thirty days [1].');
    expect(result.grounded).toBe(true);
    expect(result.citations[0]).toEqual({
      marker: 1,
      path: '/contracts/msa',
      nodeId: 'n1',
      name: 'MSA',
      chunkIndex: 3,
      textIsExact: true,
    });
    expect(result.attempts).toEqual([{ query: 'How much notice?', passages: 8 }]);
  });

  it('treats a missing `grounded` as NOT grounded', async () => {
    // An unexpected response shape must fail towards "we found nothing", never
    // towards "trust this" — `grounded` is what a caller branches on before
    // showing an answer to a stranger.
    const { api } = harness({ answer: { answer: 'something' } });

    expect((await api.ask('q', { workspaces: 'stories' })).grounded).toBe(false);
  });

  it('throws when the function reports an error instead of answering', async () => {
    const { api } = harness({ answer: { error: 'no chat model configured' } });

    await expect(
      api.ask('q', { workspaces: 'stories' }),
    ).rejects.toThrow(/no chat model configured/);
  });

  it('rejects an empty question before spending a model call', async () => {
    const { api, calls } = harness();

    await expect(api.ask('   ', { workspaces: 'stories' })).rejects.toThrow(/non-empty/);
    expect(calls.fn).toHaveLength(0);
  });
});

describe('the workspace scope', () => {
  it('ask requires it too — an answer is quoted back to whoever asked', async () => {
    const { api, calls } = harness();

    await expect(
      api.ask('what is our refund window?', { workspaces: '' }),
    ).rejects.toThrow(/workspaces/);
    expect(calls.fn, 'no model call for a question with no stated corpus').toHaveLength(0);
  });

  it('passes a multi-workspace list through verbatim', async () => {
    const { api, calls } = harness({ answer: { answer: 'a', grounded: true } });

    await api.ask('q', { workspaces: 'stories, handbook, policies' });

    expect(calls.fn[0][1]).toMatchObject({
      workspaces: 'stories, handbook, policies',
    });
  });

  it('passes a list to search as one bound parameter', async () => {
    const { api, calls } = harness();

    await api.search('q', { workspaces: 'stories, handbook' });

    // The engine parses the list; the client does not split it. Splitting here
    // would mean two parsers for one grammar, and they would drift.
    expect(calls.params[0][1]).toBe('stories, handbook');
  });

  it('accepts a glob and ALL READABLE unchanged', async () => {
    const { api, calls } = harness();

    await api.search('q', { workspaces: 'content-*' });
    await api.search('q', { workspaces: 'ALL READABLE' });

    expect(calls.params[0][1]).toBe('content-*');
    expect(calls.params[1][1]).toBe('ALL READABLE');
  });
});

describe('one implementation, both transports', () => {
  it('the HTTP client exposes search and ask, like the WebSocket one', async () => {
    // A route handler is where retrieval belongs on a public site, and a route
    // handler reaches for the HTTP client. When only the WebSocket Database had
    // these methods, following that advice led to a client that did not have
    // them — so this pins both surfaces to the same API.
    const { HttpDatabase } = await import('./http-client');
    for (const method of ['search', 'ask']) {
      assertHasMethod(HttpDatabase.prototype, method);
    }

    const { Database } = await import('./database');
    for (const method of ['search', 'ask']) {
      assertHasMethod(Database.prototype, method);
    }
  });
});

function assertHasMethod(proto: object, name: string) {
  expect(
    typeof (proto as Record<string, unknown>)[name],
    `${proto.constructor?.name ?? 'prototype'} is missing ${name}()`,
  ).toBe('function');
}
