import assert from 'node:assert/strict';
import test from 'node:test';

import { handler } from './index.js';

const PASSAGE = {
  path: '/contracts/msa',
  node_id: 'n1',
  title: 'MSA',
  chunk_index: 3,
  text: 'Either party may terminate on thirty days written notice.',
  text_is_exact: true,
};

/**
 * Stubs retrieval and the model.
 *
 * `searchFor` maps a query string to its results, so a rewritten query can be
 * made to return something the original did not — which is the whole point of
 * the loop. `completions` records every model call in order; the grader and the
 * answer both go through it, distinguished by their system prompt.
 */
function stubRaisin({ searchFor = {}, grader, model = 'openai:gpt-4o', graph } = {}) {
  const calls = { searches: [], completions: [] };

  globalThis.raisin = {
    functions: {
      // The functions the code actually calls: `call`, not `execute`.
      async call(path, args) {
        if (path.endsWith('graph-context')) return graph || { nodes: [] };
        calls.searches.push(args.query);
        const hit = searchFor[args.query];
        if (hit && hit.error) return hit;
        return { results: hit || [] };
      },
    },
    ai: {
      getDefaultModel: () => model,
      async completion(request) {
        calls.completions.push(request);
        const isGrader = request.messages[0].content.includes('You judge whether');
        if (isGrader) {
          return { content: JSON.stringify(grader || { sufficient: true, rewrite: '' }), model: request.model };
        }
        return { content: 'The notice period is 30 days [1].', model: request.model };
      },
    },
  };

  return calls;
}

test('nothing retrieved → no answer call, and it says so', async () => {
  const calls = stubRaisin({ searchFor: {} });

  const out = await handler({ question: 'what is the notice period?' });

  assert.equal(out.grounded, false);
  assert.deepEqual(out.citations, []);
  assert.equal(
    calls.completions.filter((c) => !c.messages[0].content.includes('You judge')).length,
    0,
    'a model asked to answer from an empty context answers from its own ' +
      'weights, fluently and unfalsifiably — the worst failure this has'
  );
});

test('a good first retrieval does not trigger a second search', async () => {
  const calls = stubRaisin({
    searchFor: { 'how long is the notice period?': [PASSAGE] },
    grader: { sufficient: true, rewrite: '' },
  });

  const out = await handler({ question: 'how long is the notice period?' });

  assert.equal(calls.searches.length, 1);
  assert.equal(out.attempts.length, 1);
  assert.equal(out.grounded, true);
});

test('an insufficient retrieval is rewritten and retried once', async () => {
  const calls = stubRaisin({
    searchFor: {
      'how much warning before we get kicked out?': [],
      'termination written notice period': [PASSAGE],
    },
    grader: { sufficient: false, rewrite: 'termination written notice period' },
  });

  const out = await handler({ question: 'how much warning before we get kicked out?' });

  assert.deepEqual(calls.searches, [
    'how much warning before we get kicked out?',
    'termination written notice period',
  ]);
  assert.equal(out.grounded, true);
  assert.deepEqual(
    out.attempts.map((a) => a.passages),
    [0, 1],
    'the attempt trail must show the rewrite worked, or nobody can tell ' +
      'whether the loop is earning its cost'
  );
});

test('the loop is bounded — a grader that never agrees still stops', async () => {
  const calls = stubRaisin({
    searchFor: {},
    grader: { sufficient: false, rewrite: 'another phrasing' },
  });

  await handler({ question: 'unanswerable' });

  assert.equal(
    calls.searches.length,
    2,
    'a question with no answer must not burn attempts forever'
  );
});

test('an unparseable grader degrades to plain one-shot RAG', async () => {
  const calls = stubRaisin({ searchFor: { q: [PASSAGE] } });
  raisin.ai.completion = async (request) => {
    calls.completions.push(request);
    if (request.messages[0].content.includes('You judge')) {
      return { content: 'I think it is fine?' };
    }
    return { content: 'answer [1]', model: request.model };
  };

  const out = await handler({ question: 'q' });

  assert.equal(calls.searches.length, 1, 'a broken grader must not cause a retry storm');
  assert.equal(out.grounded, true);
});

test('a grader that throws does not fail the answer', async () => {
  const calls = stubRaisin({ searchFor: { q: [PASSAGE] } });
  raisin.ai.completion = async (request) => {
    if (request.messages[0].content.includes('You judge')) {
      throw new Error('grader model unavailable');
    }
    calls.completions.push(request);
    return { content: 'answer [1]', model: request.model };
  };

  const out = await handler({ question: 'q' });

  assert.equal(out.grounded, true);
  assert.match(out.answer, /answer/);
});

test('a failed retrieval is an ERROR, not an empty answer', async () => {
  stubRaisin({ searchFor: { q: { error: 'workspace not readable' } } });

  await assert.rejects(
    () => handler({ question: 'q' }),
    /Retrieval failed: workspace not readable/,
    'functions.execute returns an { error } object rather than throwing; ' +
      'unchecked it becomes "the documents do not cover it"'
  );
});

test('passages are numbered, and the numbering is returned to the caller', async () => {
  const calls = stubRaisin({
    searchFor: { q: [PASSAGE, { ...PASSAGE, node_id: 'n2', path: '/contracts/nda' }] },
  });

  const out = await handler({ question: 'q' });

  const answerCall = calls.completions.find(
    (c) => !c.messages[0].content.includes('You judge')
  );
  assert.match(answerCall.messages[1].content, /\[1\] MSA/);
  assert.match(answerCall.messages[1].content, /\[2\] MSA/);
  assert.deepEqual(
    out.citations.map((c) => [c.marker, c.node_id]),
    [[1, 'n1'], [2, 'n2']]
  );
});

test('a preview passage is labelled as one in the prompt', async () => {
  const calls = stubRaisin({ searchFor: { q: [{ ...PASSAGE, text_is_exact: false }] } });

  await handler({ question: 'q' });

  const answerCall = calls.completions.find(
    (c) => !c.messages[0].content.includes('You judge')
  );
  assert.match(
    answerCall.messages[1].content,
    /\(preview only\)/,
    'unlabelled, the model quotes 200 truncated characters as the whole clause'
  );
});

test('the graph leg is opt-in and never fails the answer', async () => {
  const calls = stubRaisin({
    searchFor: { q: [PASSAGE] },
    graph: { error: 'relation index unavailable' },
  });

  const out = await handler({ question: 'q', use_graph: true });

  assert.equal(out.grounded, true, 'an enrichment must not trade a good answer for none');
  const answerCall = calls.completions.find(
    (c) => !c.messages[0].content.includes('You judge')
  );
  assert.ok(!answerCall.messages[1].content.includes('Related entities'));
});

test('graph context is appended when the walk finds something', async () => {
  const calls = stubRaisin({
    searchFor: { q: [PASSAGE] },
    graph: { nodes: [{ name: 'Acme Ltd', path: '/entities/acme-ltd', via: 'mentioned_in' }] },
  });

  await handler({ question: 'q', use_graph: true });

  const answerCall = calls.completions.find(
    (c) => !c.messages[0].content.includes('You judge')
  );
  assert.match(answerCall.messages[1].content, /Related entities/);
  assert.match(answerCall.messages[1].content, /Acme Ltd \(mentioned_in\)/);
});

test('no configured model is a clear error, not a silent empty answer', async () => {
  stubRaisin({ searchFor: { q: [PASSAGE] }, model: '' });

  await assert.rejects(() => handler({ question: 'q' }), /No chat model is configured/);
});

test('an empty question is rejected', async () => {
  stubRaisin({});
  await assert.rejects(() => handler({ question: '  ' }), /non-empty/);
});
