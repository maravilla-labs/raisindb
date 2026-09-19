/**
 * graph-context — seed by meaning, then expand along the graph.
 *
 * The middle column of the RAG diagram, assembled out of parts the engine
 * already has: `HYBRID_SEARCH` picks the entry points, `NEIGHBORS` walks the
 * relation index outward from each one.
 *
 * # Why this is a function and not a new SQL grammar
 *
 * `GRAPH_TABLE` takes its pattern as a STRING LITERAL, so search hits cannot be
 * passed into it as start nodes — that is the documented gap. Widening that
 * grammar is parser-and-planner surgery on the query engine's most intricate
 * operator. `NEIGHBORS(start, direction, type)` already takes a bound start
 * node and is already planned into a `NeighborsScan` over the relation index,
 * so the seeding problem is solved by calling it once per frontier node rather
 * than by teaching a second operator to accept a subquery. Multi-hop is then a
 * breadth-first loop, which is what the engine would have done internally
 * anyway.
 *
 * The cost is one query per frontier node, which is why the budget below is not
 * optional.
 *
 * # Which edges this walks
 *
 * `NEIGHBORS` reads the RELATION INDEX — the edges written by `RELATE ... TYPE
 * ... WEIGHT`, which are first-class, typed and weighted, and exist whether or
 * not either node's properties mention the other. That is the real graph, and
 * it is what `extract-entities` writes into.
 *
 * It is NOT the reference index. A reference is a property that happens to
 * point somewhere (`{ "raisin:ref": ... }`), reachable with `REFERENCES(...)`
 * — the natural, content-shaped edge. The two are different indexes answering
 * different questions, and conflating them would make this walk silently miss
 * every deliberate relation.
 *
 * Execution mode: async
 */

const SEARCH_FUNCTION = '/lib/raisin/ai/search-documents';

const DEFAULT_HOPS = 1;
const MAX_HOPS = 3;
const DEFAULT_SEED_LIMIT = 3;

/**
 * Hard ceiling on nodes visited, across all hops.
 *
 * Expansion is breadth-first over a graph nobody promised is sparse: one
 * popular node — a tag, a workspace root, a shared contact — can have thousands
 * of inbound references, and at two hops that is a query storm and a context
 * window nothing can use. The budget is a stop, not a guess, and hitting it is
 * REPORTED (`truncated: true`) rather than silently swallowed, because a
 * truncated neighbourhood that looks complete is how an agent concludes
 * something is unconnected when it simply was not looked at.
 */
const MAX_NODES = 60;

/** Neighbour queries per frontier node. Keeps one hub from eating the budget. */
const FAN_OUT_LIMIT = 25;

export async function handler(input) {
  const { query, seeds, workspaces, hops, seed_limit } = input || {};

  const depth = Math.min(
    Math.max(Number.isFinite(hops) ? Math.floor(hops) : DEFAULT_HOPS, 1),
    MAX_HOPS
  );

  const startPoints = Array.isArray(seeds) && seeds.length
    ? seeds.map((s) => ({ ref: String(s), path: String(s), name: '', node_type: '' }))
    : await seedFromSearch(query, workspaces, seed_limit);

  if (startPoints.length === 0) {
    return { seeds: [], nodes: [], count: 0, truncated: false };
  }

  // `seen` holds EVERY identity a node answers to — its id and its path — not
  // just the one we happen to be traversing by.
  //
  // A single key is not enough: a seed is addressed by path (or by whatever the
  // caller passed), while a neighbour row that points back at that same seed
  // carries its node ID. Keyed on one of those, the seed fails to match itself
  // and is emitted as its own neighbour. The same mismatch would duplicate any
  // node reached once by id and once by path.
  //
  // Deduping also keeps a diamond honest: two seeds that both link to the same
  // node must yield it once, at the SHORTER distance, or a caller ranking by
  // `hop` reads a node as further away than it is.
  const seen = new Set();
  for (const s of startPoints) {
    for (const key of [s.ref, s.path, s.node_id]) {
      if (key) seen.add(key);
    }
  }
  const collected = [];
  let frontier = startPoints.map((s) => s.ref);
  let truncated = false;

  for (let hop = 1; hop <= depth && frontier.length && !truncated; hop += 1) {
    const next = [];

    for (const from of frontier) {
      if (collected.length >= MAX_NODES) {
        truncated = true;
        break;
      }

      const neighbours = await neighboursOf(from);

      for (const n of neighbours) {
        const ref = referenceOf(n);
        const keys = [n.id, n.path].filter(Boolean);
        if (!ref || keys.some((k) => seen.has(k))) continue;
        for (const k of keys) seen.add(k);

        collected.push({
          path: n.path,
          node_id: n.id,
          name: n.name,
          node_type: n.node_type,
          hop,
          // The edge's TYPE, as RELATE recorded it — 'works_at', 'mentioned_in'.
          // This is the label on the arrow in the diagram, and an agent reading
          // the neighbourhood needs it to say HOW two things are connected.
          via: n.relation_type || '',
          weight: n.weight === undefined ? null : n.weight,
          from,
        });
        next.push(ref);

        if (collected.length >= MAX_NODES) {
          truncated = true;
          break;
        }
      }

      if (truncated) break;
    }

    frontier = next;
  }

  console.log(
    `[graph-context] ${startPoints.length} seed(s), ${depth} hop(s) → ` +
    `${collected.length} node(s)${truncated ? ' (truncated)' : ''}`
  );

  return {
    seeds: startPoints,
    nodes: collected,
    count: collected.length,
    truncated,
  };
}

/** Entry points, chosen by the same retrieval the `ask` function uses. */
async function seedFromSearch(query, workspaces, seedLimit) {
  if (!query || typeof query !== 'string' || !query.trim()) {
    throw new Error('Either `query` or a non-empty `seeds` array is required');
  }

  const limit = Number.isFinite(seedLimit) ? Math.floor(seedLimit) : DEFAULT_SEED_LIMIT;
  // `functions.call`, not `functions.execute` — the latter is the AI-tool-call
  // form and needs a tool-call node to stamp, which this has none of.
  const found = await raisin.functions.call(SEARCH_FUNCTION, {
    query: query.trim(),
    workspaces,
    limit: Math.max(limit, 1),
  });

  // An { error } object, not a throw — unchecked it becomes "nothing is
  // connected to this", which reads as a fact about the graph.
  if (found && found.error) {
    throw new Error(`Seed search failed: ${found.error}`);
  }

  const bySeed = new Map();
  for (const hit of (found && found.results) || []) {
    // Several passages of one document are one seed.
    if (!bySeed.has(hit.path)) {
      bySeed.set(hit.path, {
        // Traverse by id, for the reason `referenceOf` documents.
        ref: hit.node_id || hit.path,
        path: hit.path,
        node_id: hit.node_id,
        name: hit.title || '',
        node_type: '',
      });
    }
  }
  return [...bySeed.values()].slice(0, Math.max(limit, 1));
}

/**
 * One hop out of a node, in both directions.
 *
 * `BOTH` because relevance is not directional: a contract that references a
 * party and a party that references a contract are equally worth reading, and
 * which way the reference happens to point is a modelling accident.
 */
async function neighboursOf(reference) {
  const rows = await raisin.sql.query(
    `SELECT id, path, name, node_type, relation_type, weight
     FROM NEIGHBORS($1, 'BOTH', NULL)
     LIMIT ${FAN_OUT_LIMIT}`,
    [reference]
  );
  return rows || [];
}

/**
 * How a row is addressed on the NEXT hop.
 *
 * The node ID, not the path. `NEIGHBORS` resolves a BARE path inside the
 * default workspace only, and a neighbour row does not say which workspace it
 * came from — so continuing a walk by path silently searched the wrong
 * workspace and returned nothing, which reads as "this branch of the graph
 * ends here". An id needs no workspace to resolve, so the walk stays correct
 * across workspaces. The path is still carried on the result, for display and
 * for citations.
 */
function referenceOf(row) {
  if (!row) return '';
  return row.id || row.path || '';
}
