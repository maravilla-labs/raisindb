/**
 * Retrieval: finding passages, and answering from them.
 *
 * # Why these are methods and not `functions().invoke('ask', …)`
 *
 * `invoke` is the escape hatch — the way to call something the SDK does not
 * model. Reaching for it here would cost three things that matter more than the
 * line of code it saves:
 *
 *   1. **The name is a string resolved at run time.** `invoke('ask')` resolves
 *      by function NAME on the server; a typo, a rename or a package that has
 *      not been installed yet is a 404 at run time, in production, with nothing
 *      in the type checker to catch it.
 *   2. **Nobody owns the argument and response shape.** Every call site
 *      rebuilds `{ question, workspaces, limit }` from documentation and
 *      re-derives what a citation looks like. That is the drift this codebase
 *      keeps paying for elsewhere — several hand-maintained copies of one
 *      shape, differing slightly, none of them wrong enough to notice.
 *   3. **It freezes the transport.** A call site that names a function can
 *      never be moved to an endpoint, a cache or a different implementation
 *      without breaking every caller. Behind a method, that is an internal
 *      detail.
 *
 * So `search()` and `ask()` are the surface, and how they are served is ours to
 * change. `functions().invoke(...)` remains, for functions the caller wrote.
 *
 * # Why `search()` builds SQL rather than calling a search endpoint
 *
 * Because the SQL surface is the complete one — scope, granularity, weights,
 * distance ceilings, the chunk text and its provenance — and it is the one the
 * engine tests. A second wire format for search would be a second thing to keep
 * in step with the first. What matters is that the query shape is written ONCE,
 * here, instead of in every application that wants site search.
 */

import type { SqlResult } from './protocol';

/** One matching passage. */
export interface SearchPassage {
  /** Path of the node the passage came from. */
  path: string;
  /** Node id, stable across renames — cite with this plus `chunkIndex`. */
  nodeId: string;
  /** The node's name, for display. */
  name: string;
  /** Which chunk of the document matched. `0` for an unchunked node. */
  chunkIndex: number;
  /** The passage itself. */
  text: string;
  /**
   * False when only a short preview of the passage was available.
   *
   * Use it to point a reader at the document; do not quote it as if it were the
   * whole passage.
   */
  textIsExact: boolean;
  /** Fused rank score. Comparable within one result set, not across queries. */
  score: number;
}

export interface SearchOptions {
  /**
   * Where to look: a workspace name, a comma-separated list, a glob such as
   * `content-*`, or `'ALL READABLE'` for everything the caller may read.
   *
   * Required by the engine and deliberately not defaulted here for a public
   * surface: an unscoped search on a website is how draft content reaches a
   * visitor. Pass `'ALL READABLE'` explicitly when that is what you mean.
   */
  workspaces: string;
  /** How many passages. Defaults to 8. */
  limit?: number;
  /** Optional vector-distance ceiling, to tighten precision. */
  maxDistance?: number;
}

/** An answer, and the passages it was drawn from. */
export interface Answer {
  /** The answer text. Empty when `grounded` is false. */
  answer: string;
  /**
   * False when nothing relevant was retrieved.
   *
   * The model is NOT called in that case — it is not that the model declined,
   * it is that it was never asked. A model handed an empty context answers from
   * its own training, fluently, with nothing in the reply marking it invented.
   */
  grounded: boolean;
  /** The passages given to the model, numbered as the answer cites them. */
  citations: Citation[];
  /**
   * One entry per retrieval attempt. A second entry means the first search was
   * graded insufficient and the query was rewritten.
   */
  attempts: { query: string; passages: number }[];
  /** The model that produced the answer. */
  model: string;
}

export interface Citation {
  /** The number used in the answer text: `[1]`, `[2]`. */
  marker: number;
  path: string;
  nodeId: string;
  name: string;
  chunkIndex: number;
  textIsExact: boolean;
}

export interface AskOptions {
  /** Where to look. Defaults to everything the caller may read. */
  workspaces?: string;
  /** How many passages to ground the answer in. Defaults to 8. */
  limit?: number;
  /** Model id, as `slug:model`. Defaults to the tenant's configured chat model. */
  model?: string;
  /** Also bring in the graph neighbourhood of the question's subject. */
  useGraph?: boolean;
}

/** The function that serves {@link SearchApi.ask}. */
const ASK_FUNCTION = 'ask';

/** Passages to return when the caller does not say. */
const DEFAULT_LIMIT = 8;

/**
 * Upper bound on `limit`.
 *
 * Each leg over-fetches many times the limit before row-level security filters,
 * so a caller asking for a thousand passages is asking the engine for tens of
 * thousands of candidates to throw away.
 */
const MAX_LIMIT = 50;

type SqlRunner = (sql: string, params: unknown[]) => Promise<SqlResult>;
type FunctionRunner = (
  name: string,
  input: Record<string, unknown>,
) => Promise<unknown>;

export class SearchApi {
  constructor(
    private runSql: SqlRunner,
    private runFunction: FunctionRunner,
  ) {}

  /**
   * Find the passages that match a query, by meaning and by keyword at once.
   *
   * No model is involved, so this is the call for a search box, a "related
   * content" list, or anything assembling its own context.
   */
  async search(query: string, options: SearchOptions): Promise<SearchPassage[]> {
    if (!query || !query.trim()) {
      throw new Error('search(query) requires a non-empty query');
    }
    if (!options || !options.workspaces || !options.workspaces.trim()) {
      throw new Error(
        "search() requires `workspaces` — name a workspace, a list, a glob, or " +
          "'ALL READABLE'. An unscoped search on a public site is how draft " +
          'content reaches a visitor.',
      );
    }

    const limit = Math.min(
      Math.max(Math.floor(options.limit ?? DEFAULT_LIMIT), 1),
      MAX_LIMIT,
    );

    // Only numbers this method produced are interpolated. The query text and
    // the scope are BOUND — both are caller-controlled, and on a website the
    // query text is whatever a stranger typed into a search box.
    const distance = Number.isFinite(options.maxDistance as number)
      ? `, max_distance => ${Number(options.maxDistance)}`
      : '';

    const sql = `
      SELECT node_id, path, name, score, chunk_index, chunk_text, chunk_text_source
      FROM HYBRID_SEARCH($1, ${limit},
                         workspaces => $2,
                         granularity => 'chunk'${distance})
    `;

    const result = await this.runSql(sql, [query.trim(), options.workspaces.trim()]);
    const rows = (result?.rows ?? []) as Record<string, unknown>[];

    return rows
      .map((row) => ({
        path: String(row.path ?? ''),
        nodeId: String(row.node_id ?? ''),
        name: String(row.name ?? ''),
        // `0` is a real chunk index — a document that was never split — so it
        // must not be collapsed into "missing" by a falsy check.
        chunkIndex: row.chunk_index == null ? 0 : Number(row.chunk_index),
        text: String(row.chunk_text ?? ''),
        textIsExact: row.chunk_text_source === 'exact',
        score: Number(row.score ?? 0),
      }))
      .filter((passage) => passage.text.length > 0);
  }

  /**
   * Answer a question from the stored content, with citations.
   *
   * Retrieves, judges whether what came back answers the question, rewrites the
   * query once if it does not, and answers from those passages only.
   */
  async ask(question: string, options: AskOptions = {}): Promise<Answer> {
    if (!question || !question.trim()) {
      throw new Error('ask(question) requires a non-empty question');
    }

    const raw = (await this.runFunction(ASK_FUNCTION, {
      question: question.trim(),
      workspaces: options.workspaces,
      limit: options.limit,
      model: options.model,
      use_graph: options.useGraph,
    })) as Record<string, unknown> | null;

    if (raw && typeof raw === 'object' && 'error' in raw && raw.error) {
      throw new Error(`ask failed: ${String(raw.error)}`);
    }

    const citations = Array.isArray(raw?.citations) ? raw!.citations : [];

    return {
      answer: String(raw?.answer ?? ''),
      // Absent counts as NOT grounded. This flag is the one a caller branches
      // on before showing an answer, so an unexpected response shape must fail
      // towards "we found nothing", never towards "trust this".
      grounded: raw?.grounded === true,
      citations: (citations as Record<string, unknown>[]).map((c) => ({
        marker: Number(c.marker ?? 0),
        path: String(c.path ?? ''),
        nodeId: String(c.node_id ?? ''),
        name: String(c.title ?? ''),
        chunkIndex: c.chunk_index == null ? 0 : Number(c.chunk_index),
        textIsExact: c.text_is_exact === true,
      })),
      attempts: Array.isArray(raw?.attempts)
        ? (raw!.attempts as Record<string, unknown>[]).map((a) => ({
            query: String(a.query ?? ''),
            passages: Number(a.passages ?? 0),
          }))
        : [],
      model: String(raw?.model ?? ''),
    };
  }
}
