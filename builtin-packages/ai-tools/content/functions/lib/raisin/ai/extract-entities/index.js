/**
 * extract-entities — build the graph out of the documents.
 *
 * The index-time step of the Graph RAG column: read a document's extracted
 * text, have a model name the entities and the relations between them, and
 * write those as nodes joined by real, typed graph edges.
 *
 * # Why this is a function and not a Rust job
 *
 * Extraction is a judgement call with a model and a prompt behind it — which
 * entities are worth having, how finely to split them, what a relation is
 * called. That is policy, and policy lives in a function, the same way
 * captioning does and for the same reason. Core owes it the deterministic half,
 * and core already provides that: the asset pipeline extracts the text, and the
 * graph this writes is indexed by the engine with no new path.
 *
 * # These are RELATE edges, not reference properties
 *
 * RaisinDB has two different kinds of link, and this uses the explicit one:
 *
 *   - A **reference** is a property that points somewhere
 *     (`{ "raisin:ref": "/path", "raisin:workspace": "ws" }`). It is the
 *     natural, content-shaped edge — an article's author field, a page's hero
 *     image — and you query it with `REFERENCES(...)`. It exists because the
 *     content says so.
 *   - A **relation** is `RELATE FROM ... TO ... TYPE ... WEIGHT ...`: a
 *     first-class, typed, weighted edge in the relation index, independent of
 *     either node's properties. You query it with `NEIGHBORS` and
 *     `GRAPH_TABLE`. It exists because someone asserted it.
 *
 * An extracted knowledge graph is the second kind. "Acme employs Dana" is an
 * assertion about the world, not a field on a document, and `works_at` needs to
 * be the LABEL ON THE EDGE — which is exactly what a reference property cannot
 * carry and what `TYPE` is for. Writing these as references would also bury
 * them among the content's own links, so a traversal could no longer tell
 * "the document mentions this" from "the author wrote this".
 *
 * # Idempotency is the whole cost model
 *
 * Every run costs a model call, and this is meant to be triggered by document
 * changes. Without a guard, an install that re-saves nodes — a sync, a
 * republish, a bulk edit — pays for an extraction per save forever, and mints a
 * node revision each time, which triggers reindexing, which is another write.
 * So a run records a fingerprint of the text it read, and a later run that
 * finds the same fingerprint does nothing at all. Same shape as the spec hash
 * that makes a steady-state embedding run write nothing.
 *
 * Execution mode: async
 */

const DEFAULT_WORKSPACE = 'content';

/** Where entities and run markers live, under the entity workspace root. */
const ENTITY_FOLDER = '/entities';
const RUN_FOLDER = '/entities/.runs';

/** The edge from an entity back to a document that names it. */
const MENTION_EDGE = 'mentioned_in';

/** Text handed to the model. Enough for the entities; not a whole contract. */
const MAX_TEXT = 12000;

/** Entities accepted from one document. A model asked for "all" will oblige. */
const MAX_ENTITIES = 40;

const SYSTEM_PROMPT = [
  'You extract a knowledge graph from a document.',
  '',
  'Return ONLY a JSON object, with no prose and no code fence:',
  '{"entities":[{"name":"...","type":"person|organization|place|product|concept|event"}],',
  ' "relations":[{"from":"...","to":"...","type":"works_at"}]}',
  '',
  'Rules:',
  '- Only entities the document actually names. Never invent one.',
  '- `name` must be the entity as written in the document, not a description.',
  '- Merge obvious duplicates ("Acme", "Acme Ltd") into the fuller form.',
  '- `from` and `to` in relations must each equal a `name` in entities.',
  '- A relation type is a short lower_snake_case verb phrase.',
  '- Prefer fewer, load-bearing entities over an exhaustive list.',
].join('\n');

export async function handler(input) {
  const { path, workspace, entity_workspace, model, force } = input || {};

  if (!path || typeof path !== 'string') {
    throw new Error('A `path` is required');
  }

  const ws = workspace || DEFAULT_WORKSPACE;
  const entityWs = entity_workspace || ws;

  const doc = await raisin.nodes.get(ws, path);
  if (!doc) {
    throw new Error(`Node not found: ${ws}:${path}`);
  }

  const text = documentText(doc);
  if (!text) {
    console.log(`[extract-entities] ${path} has no text to read`);
    return { skipped: true, entities: [], relations: [], count: 0 };
  }

  const fingerprint = fingerprintOf(text);
  const runPath = `${RUN_FOLDER}/${markerName(doc, path)}`;

  if (!force && (await alreadyExtracted(entityWs, runPath, fingerprint))) {
    console.log(`[extract-entities] ${path} unchanged (${fingerprint}), skipping`);
    return { skipped: true, entities: [], relations: [], count: 0 };
  }

  const chosenModel = model || raisin.ai.getDefaultModel('chat');
  if (!chosenModel) {
    throw new Error(
      'No chat model is configured for this tenant, and none was passed as `model`.'
    );
  }

  const response = await raisin.ai.completion({
    model: chosenModel,
    messages: [
      { role: 'system', content: SYSTEM_PROMPT },
      {
        role: 'user',
        content:
          `Document: ${doc.properties?.title || doc.name || path}\n\n` +
          text.slice(0, MAX_TEXT),
      },
    ],
  });

  const parsed = parseGraph(response && response.content);
  const entities = parsed.entities.slice(0, MAX_ENTITIES);

  if (entities.length === 0) {
    // Still stamp the run: a document that genuinely names nothing must not be
    // re-sent to the model on every trigger.
    await stampRun(entityWs, runPath, path, fingerprint, 0);
    console.log(`[extract-entities] ${path} named no entities`);
    return { skipped: false, entities: [], relations: [], count: 0 };
  }

  await ensureFolder(entityWs, '', 'entities', 'Entities');
  await ensureFolder(entityWs, ENTITY_FOLDER, '.runs', 'Extraction runs');

  const byName = new Map();
  const written = [];

  for (const entity of entities) {
    const result = await upsertEntity(entityWs, entity);
    byName.set(entity.name.toLowerCase(), result.path);
    written.push({ ...entity, path: result.path, created: result.created });

    // entity —mentioned_in→ document. This is the edge that makes the whole
    // feature work: from a document you reach its entities, and from an entity
    // you reach EVERY document that names it, which is the connection a vector
    // search over one document can never make.
    await relate(entityWs, result.path, ws, path, MENTION_EDGE);
  }

  // Entity-to-entity edges. One naming an entity we did not write is dropped:
  // RELATE to a node that does not exist would either fail or create a dangling
  // edge that traverses to nothing and reports no fault.
  const relations = [];
  for (const rel of parsed.relations) {
    const fromPath = byName.get(String(rel.from || '').toLowerCase());
    const toPath = byName.get(String(rel.to || '').toLowerCase());
    if (!fromPath || !toPath || fromPath === toPath) continue;

    const type = edgeType(rel.type);
    await relate(entityWs, fromPath, entityWs, toPath, type);
    relations.push({ from: rel.from, to: rel.to, type });
  }

  await stampRun(entityWs, runPath, path, fingerprint, written.length);

  console.log(
    `[extract-entities] ${path} → ${written.length} entity(ies), ` +
    `${relations.length} relation(s)`
  );

  return { skipped: false, entities: written, relations, count: written.length };
}

/**
 * Assert one typed edge.
 *
 * `RELATE` is idempotent on `(from, to, type)` — re-asserting an existing edge
 * is a no-op rather than a duplicate — which is what lets this function be
 * re-run without `UNRELATE`-ing first.
 *
 * The type is interpolated rather than bound because it reaches a SQL keyword
 * position; `edgeType` restricts it to `[a-z0-9_]` for exactly that reason. The
 * paths are bound.
 */
async function relate(fromWs, fromPath, toWs, toPath, type) {
  await raisin.sql.execute(
    `RELATE FROM path=$1 IN WORKSPACE $2
            TO path=$3 IN WORKSPACE $4
            TYPE '${type}'`,
    [fromPath, fromWs, toPath, toWs]
  );
}

/**
 * A relation type safe to place in the statement.
 *
 * Anything a model invents is reduced to lower_snake_case and truncated. A
 * quote character here would not merely break the statement, it would end the
 * literal — so this is the injection boundary, and it fails to a safe constant
 * rather than passing anything through.
 */
function edgeType(raw) {
  const cleaned = String(raw || '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, 48);
  return cleaned || 'related_to';
}

/**
 * The text to read.
 *
 * `__extracted_text` is the engine-owned artifact an asset carries — a PDF core
 * read, or markdown a converter plugin handed back through
 * `raisin.assets.setExtractedText`. Ordinary content nodes have no such
 * artifact, so their own body is used instead.
 */
function documentText(node) {
  const props = node.properties || {};
  const candidate =
    props.__extracted_text || props.content || props.body || props.text || '';
  return typeof candidate === 'string' && candidate.trim() ? candidate : '';
}

/**
 * A cheap, stable fingerprint of the text.
 *
 * djb2, because the function runtime has no crypto binding and this is change
 * DETECTION, not integrity: the cost of a collision is one skipped extraction,
 * and the cost of not having it is a model call on every save forever.
 */
function fingerprintOf(text) {
  let hash = 5381;
  for (let i = 0; i < text.length; i += 1) {
    hash = ((hash << 5) + hash + text.charCodeAt(i)) >>> 0;
  }
  return `${hash.toString(16)}-${text.length}`;
}

/** A filesystem-safe marker name that is stable for one document. */
function markerName(doc, path) {
  const raw = doc.id || path;
  return String(raw).replace(/[^a-zA-Z0-9_-]/g, '_').slice(0, 120);
}

async function alreadyExtracted(workspace, runPath, fingerprint) {
  const marker = await safeGet(workspace, runPath);
  return !!marker && marker.properties?.source_fingerprint === fingerprint;
}

async function stampRun(workspace, runPath, sourcePath, fingerprint, count) {
  const name = runPath.split('/').pop();
  const props = {
    source_path: sourcePath,
    source_fingerprint: fingerprint,
    entity_count: count,
    extracted_at: new Date().toISOString(),
  };

  if (await safeGet(workspace, runPath)) {
    await raisin.nodes.update(workspace, runPath, { properties: props });
  } else {
    await raisin.nodes.create(workspace, RUN_FOLDER, {
      name,
      node_type: 'raisin:Folder',
      properties: props,
    });
  }
}

/**
 * Create the entity, or return the one already there.
 *
 * Convergence is the point of the whole feature: two documents naming the same
 * person must land on ONE node, so that walking out from that person reaches
 * both. The per-document link is then an edge, not a property, so an entity
 * named by a thousand documents stays a small node with a thousand edges rather
 * than a node with a thousand-element array.
 */
async function upsertEntity(entityWs, entity) {
  const name = slugify(entity.name);
  const entityPath = `${ENTITY_FOLDER}/${name}`;

  if (await safeGet(entityWs, entityPath)) {
    return { path: entityPath, created: false };
  }

  await raisin.nodes.create(entityWs, ENTITY_FOLDER, {
    name,
    node_type: 'raisin:Folder',
    properties: {
      title: entity.name,
      entity_type: entity.type || 'concept',
    },
  });
  return { path: entityPath, created: true };
}

/**
 * The model's JSON, defensively.
 *
 * Models fence JSON in markdown, prepend "Here is the graph:", and occasionally
 * emit nothing usable. A parse failure must not take the trigger down — an
 * unparseable answer means no entities from this document, not an error the
 * caller has to handle.
 */
function parseGraph(content) {
  const empty = { entities: [], relations: [] };
  if (!content || typeof content !== 'string') return empty;

  const start = content.indexOf('{');
  const end = content.lastIndexOf('}');
  if (start === -1 || end <= start) return empty;

  let parsed;
  try {
    parsed = JSON.parse(content.slice(start, end + 1));
  } catch (err) {
    console.log(`[extract-entities] model did not return JSON: ${err.message}`);
    return empty;
  }

  const entities = Array.isArray(parsed.entities) ? parsed.entities : [];
  const relations = Array.isArray(parsed.relations) ? parsed.relations : [];

  return {
    entities: entities
      .filter((e) => e && typeof e.name === 'string' && e.name.trim())
      .map((e) => ({ name: e.name.trim(), type: String(e.type || 'concept') })),
    relations: relations.filter((r) => r && r.from && r.to),
  };
}

function slugify(name) {
  return (
    String(name)
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 80) || 'entity'
  );
}

async function safeGet(workspace, path) {
  try {
    return await raisin.nodes.get(workspace, path);
  } catch (_) {
    return null;
  }
}

async function ensureFolder(workspace, parentPath, name, title) {
  const full = `${parentPath}/${name}`;
  if (await safeGet(workspace, full)) return;
  try {
    await raisin.nodes.create(workspace, parentPath || '/', {
      name,
      node_type: 'raisin:Folder',
      properties: { title },
    });
  } catch (err) {
    // Another concurrent run may have created it; that is success, not failure.
    console.log(`[extract-entities] folder note: ${err.message}`);
  }
}
