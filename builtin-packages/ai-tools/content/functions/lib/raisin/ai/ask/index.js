/**
 * ask — retrieve, grade, retry, then answer with citations.
 *
 * The last box of the classic RAG diagram plus the loop from the agentic one,
 * and the only parts the engine does not do itself: retrieval is a SQL table
 * function, everything here is a model call with a prompt behind it.
 *
 * # Why this lives in a function and not in the engine
 *
 * Everything below the seam is deterministic — same document, same chunks, same
 * vectors — and belongs in Rust, where it is guarded by a spec hash. Everything
 * above it is a judgement call: how many passages to show, how to say "I don't
 * know", when a retrieval is good enough, which model to spend. That is policy,
 * it changes per product, and it is exactly what a function is for. Putting the
 * prompt in the engine would freeze it into a release cycle.
 *
 * # The loop
 *
 *   retrieve → grade → (rewrite → retrieve again) → answer
 *
 * The grader is a separate, cheap call that sees the QUESTION and the retrieved
 * passages and answers with JSON: does this answer it, and if not, what would a
 * better search be? The rewrite matters more than it sounds — a user asks "how
 * long do I have to give notice", the documents say "termination requires
 * thirty days written notice", and the lexical leg matches nothing while the
 * vector leg matches vaguely. One rewrite to the document's own vocabulary is
 * often the whole difference between an answer and "not covered".
 *
 * It is bounded at `MAX_ATTEMPTS`, and a grader that cannot be parsed counts as
 * "good enough" — a broken grader must degrade to plain RAG, never to a loop
 * that keeps paying for retrievals.
 *
 * # Grounding is enforced here, not hoped for
 *
 *   1. Retrieval returns nothing → `grounded: false` and a plain "not found"
 *      WITHOUT calling the model. A model asked to answer from an empty context
 *      answers from its own weights, fluently and unfalsifiably, which is the
 *      single worst failure a RAG system has.
 *   2. Passages are numbered, and the numbering is returned, so a caller can
 *      resolve `[2]` to a path and a chunk index rather than trusting prose.
 *   3. A passage the engine could only give us as a PREVIEW is labelled as one,
 *      or the model quotes 200 truncated characters as a whole clause.
 *
 * Execution mode: async
 */

const SEARCH_FUNCTION = '/lib/raisin/ai/search-documents';
const GRAPH_FUNCTION = '/lib/raisin/ai/graph-context';

const DEFAULT_LIMIT = 8;

/**
 * Retrievals per question, including the first.
 *
 * Two. Each attempt costs a search plus a grader call, and the second attempt
 * is where nearly all the benefit is: it applies the document's own vocabulary.
 * A third rarely finds what two did not, and the cost is paid on every question
 * that genuinely has no answer — which is exactly the case the loop must not
 * make expensive.
 */
const MAX_ATTEMPTS = 2;

const SYSTEM_PROMPT = [
  'You answer questions using only the numbered passages provided.',
  '',
  'Rules:',
  '- Use only what the passages say. Do not add facts from your own knowledge,',
  '  even when you are confident they are correct.',
  '- Cite the passages you used as [1], [2], and so on, immediately after the',
  '  claim they support.',
  '- If the passages do not answer the question, say plainly that the available',
  '  documents do not cover it. That is a correct and useful answer, not a',
  '  failure.',
  '- A passage marked (preview only) is truncated. You may use it to point the',
  '  reader at the document, but never quote it as if it were complete.',
  '- Answer in the language the question was asked in.',
].join('\n');

const GRADER_PROMPT = [
  'You judge whether retrieved passages can answer a question.',
  '',
  'Return ONLY JSON, no prose and no code fence:',
  '{"sufficient": true|false, "rewrite": "a better search query, or empty"}',
  '',
  'Rules:',
  '- `sufficient` is true when the passages contain the facts needed. They do',
  '  not have to be well written or complete documents.',
  '- When false, `rewrite` must be a query phrased in the vocabulary the',
  '  documents themselves appear to use, not a rephrasing of the question.',
  '- If the passages are simply about another subject, say false and rewrite.',
].join('\n');

export async function handler(input) {
  const { question, workspaces, limit, model, use_graph } = input || {};

  if (!question || typeof question !== 'string' || !question.trim()) {
    throw new Error('A non-empty `question` is required');
  }

  const asked = question.trim();
  const wanted = Number.isFinite(limit) ? Math.floor(limit) : DEFAULT_LIMIT;

  const chosenModel = model || raisin.ai.getDefaultModel('chat');
  if (!chosenModel) {
    throw new Error(
      'No chat model is configured for this tenant, and none was passed as ' +
      '`model`. Configure one in the AI settings.'
    );
  }

  let searchFor = asked;
  let passages = [];
  const attempts = [];

  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt += 1) {
    passages = await retrieve(searchFor, workspaces, wanted);
    attempts.push({ query: searchFor, passages: passages.length });

    if (attempt === MAX_ATTEMPTS) break;

    const verdict = await grade(chosenModel, asked, passages);
    if (verdict.sufficient || !verdict.rewrite) break;

    console.log(`[ask] rewriting "${searchFor}" → "${verdict.rewrite}"`);
    searchFor = verdict.rewrite;
  }

  if (passages.length === 0) {
    console.log(`[ask] no passages for "${asked.slice(0, 80)}"`);
    return {
      answer:
        'I could not find anything in the available documents that answers ' +
        'this question.',
      grounded: false,
      citations: [],
      attempts,
      model: '',
    };
  }

  const citations = passages.map((p, i) => ({
    marker: i + 1,
    path: p.path,
    node_id: p.node_id,
    title: p.title,
    chunk_index: p.chunk_index,
    text_is_exact: p.text_is_exact,
  }));

  let context = passages
    .map((p, i) => {
      const label = p.text_is_exact ? '' : ' (preview only)';
      const where = p.title ? `${p.title} — ${p.path}` : p.path;
      return `[${i + 1}] ${where}${label}\n${p.text}`;
    })
    .join('\n\n');

  // The graph leg, off by default. It answers a different question — how things
  // relate — and costs a walk, so it is opt-in rather than always paid for.
  if (use_graph) {
    const related = await neighbourhood(asked, workspaces);
    if (related) context += `\n\nRelated entities:\n${related}`;
  }

  const response = await raisin.ai.completion({
    model: chosenModel,
    messages: [
      { role: 'system', content: SYSTEM_PROMPT },
      { role: 'user', content: `Question: ${asked}\n\nPassages:\n\n${context}` },
    ],
  });

  const answer = (response && response.content) || '';

  console.log(
    `[ask] "${asked.slice(0, 80)}" → ${passages.length} passage(s) in ` +
    `${attempts.length} attempt(s), ${answer.length} chars from ${chosenModel}`
  );

  return {
    answer,
    grounded: true,
    citations,
    attempts,
    model: (response && response.model) || chosenModel,
  };
}

/** One retrieval. Throws on a failed search rather than reporting no passages. */
async function retrieve(query, workspaces, limit) {
  // `functions.call`, NOT `functions.execute`. They run the same job, but
  // `execute` is the AI-TOOL-CALL form: it first writes 'running' onto an
  // `raisin:AIToolCall` node taken from its third argument, so calling it
  // outside a tool-call context fails with "Node not found" before the callee
  // ever runs. `call` is the plain function-to-function form.
  const found = await raisin.functions.call(SEARCH_FUNCTION, {
    query,
    workspaces,
    limit,
  });

  // It returns an { error } OBJECT rather than throwing, so a failed retrieval
  // must be checked for explicitly. Left unchecked it becomes "no passages",
  // and this function would confidently report that the documents do not cover
  // the question when in fact nothing was searched.
  if (found && found.error) {
    throw new Error(`Retrieval failed: ${found.error}`);
  }
  return (found && found.results) || [];
}

/**
 * Does this retrieval answer the question, and if not, what should we search
 * instead?
 *
 * Fails to `sufficient: true`. A grader that errors, times out or returns
 * something unparseable must degrade this to ordinary one-shot RAG — the
 * alternative is a question that cannot be answered burning every attempt on a
 * grader that was never going to agree.
 */
async function grade(model, question, passages) {
  const good = { sufficient: true, rewrite: '' };

  // An EMPTY retrieval still goes to the grader, and that is not an oversight.
  // It is the case where a rewrite is worth the most — the query matched
  // nothing at all — and it is the case an early return gets wrong: returning
  // `{ sufficient: false, rewrite: '' }` here reads as "retry" but carries no
  // query to retry WITH, so the loop breaks immediately and the one question
  // that most needed a second phrasing gets only one.
  const summary = passages.length
    ? passages
        .map((p, i) => `[${i + 1}] ${p.title || p.path}\n${(p.text || '').slice(0, 400)}`)
        .join('\n\n')
    : '(no passages matched this query at all)';

  let response;
  try {
    response = await raisin.ai.completion({
      model,
      messages: [
        { role: 'system', content: GRADER_PROMPT },
        { role: 'user', content: `Question: ${question}\n\nPassages:\n\n${summary}` },
      ],
    });
  } catch (err) {
    console.log(`[ask] grader unavailable, proceeding: ${err.message}`);
    return good;
  }

  const content = (response && response.content) || '';
  const start = content.indexOf('{');
  const end = content.lastIndexOf('}');
  if (start === -1 || end <= start) return good;

  try {
    const parsed = JSON.parse(content.slice(start, end + 1));
    return {
      sufficient: parsed.sufficient !== false,
      rewrite: typeof parsed.rewrite === 'string' ? parsed.rewrite.trim() : '',
    };
  } catch (_) {
    return good;
  }
}

/** A compact rendering of the graph neighbourhood, or '' when there is none. */
async function neighbourhood(question, workspaces) {
  const graph = await raisin.functions.call(GRAPH_FUNCTION, {
    query: question,
    workspaces,
    hops: 1,
  });

  // A graph walk is an ENRICHMENT here. Failing the whole answer because the
  // relation index had nothing to say would trade a good answer for no answer.
  if (!graph || graph.error || !Array.isArray(graph.nodes) || !graph.nodes.length) {
    return '';
  }

  return graph.nodes
    .slice(0, 20)
    .map((n) => `- ${n.name || n.path}${n.via ? ` (${n.via})` : ''}`)
    .join('\n');
}
