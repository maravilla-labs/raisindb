/**
 * Skills — which raisin:Skill nodes an agent has been given, and the exact
 * INDEX they add to its system prompt.
 *
 * A skill is the Agent Skills / SKILL.md shape: a `name`, a one-sentence
 * `description`, and a Markdown `body` of instructions. The prompt carries only
 * the INDEX — one line per skill — and the body is read ON DEMAND through the
 * `load-skill` tool, which refuses a skill the caller was not given.
 *
 * ONE RULE, TWO RUNTIMES. The chat loop (agent-handler, agent-continue-handler)
 * and the load-skill function run this file. The flow runtime (agent and chat
 * steps) runs a Rust port in
 * crates/raisin-functions/src/execution/flow_callbacks_factory/skills.rs, and
 * that crate's parity test EXECUTES this module in QuickJS and asserts the two
 * agree on selectSkills, skillIndexText, composeInstructionTail and appendTail.
 * That is why this module:
 *   - never touches `raisin` at module top level (QuickJS has none there);
 *   - keeps the rule PURE — select and compose are data in, data out. Only
 *     `loadSkillGrant` does IO, and it decides nothing;
 *   - counts characters in Unicode scalar values (Array.from), never .length,
 *     and strips whitespace by an explicit ASCII set, never trim(): both are the
 *     places a JS and a Rust implementation silently disagree;
 *   - orders strings by CODE POINT, never by `<` on UTF-16, so a sort agrees
 *     with Rust's byte order on UTF-8.
 * Change the rule here and in skills.rs together, or the parity test fails.
 *
 * WHICH SKILLS, in this order — the FIRST occurrence of a name wins:
 *   agent         — the agent's `skills:` references, in declared order;
 *   step          — a workflow step's own `skills:`, added for that step only;
 *   installation  — functions:/local/skills/*, the operator's, by name;
 *   package       — functions:/skills/*, what packages ship, by name.
 * The two global layers apply unless the agent says `global_skills: false`.
 * A local skill REPLACES the package one of the same name, and a local skill
 * with `enabled: false` and a valid name MASKS it — the per-installation
 * opt-out. An EXPLICIT reference that is disabled or unusable is simply absent
 * and masks nothing.
 *
 * A skill is USABLE when it is a raisin:Skill node, not `enabled: false`, its
 * `name` matches the Agent Skills grammar (^[a-z0-9]+(-[a-z0-9]+)*$, at most 64
 * characters), and its description says something once whitespace is collapsed.
 *
 * ABSENT MEANS ABSENT: nothing resolved and no rules is the empty tail, and an
 * empty tail leaves the system prompt byte for byte as it was.
 */

export const SKILL_NODE_TYPE = 'raisin:Skill';

/* The builtin tool that reads a skill's body, and where it lives. */
export const LOAD_SKILL_TOOL_NAME = 'load-skill';
export const LOAD_SKILL_FUNCTION = { workspace: 'functions', path: '/lib/raisin/ai/load-skill' };

/* THE CAPS, in Unicode scalar values. The index goes out on EVERY model call —
 * for a builder that is every tool round — so it is a view, never the grant. */
export const SKILL_INDEX_MAX_LINES = 40;
export const SKILL_INDEX_MAX_CHARS = 6000;
export const SKILL_DESCRIPTION_MAX_CHARS = 200;
export const SKILL_BODY_MAX_CHARS = 20000;

export const SKILL_NAME_MAX_CHARS = 64;

/* The two global layers, shaped like Studio's /apps/libs -> /apps/local. */
export const GLOBAL_SKILL_ROOTS = {
  installation: { workspace: 'functions', path: '/local/skills' },
  package: { workspace: 'functions', path: '/skills' },
};

const INDEX_HEADER = '\n\n## Skills\nLoad a skill with the load-skill tool before doing work its description covers; its instructions then apply.';

/* The whitespace set both runtimes agree on: ASCII space, tab, CR, LF. */
function isAsciiSpace(ch) {
  return ch === ' ' || ch === '\t' || ch === '\r' || ch === '\n';
}

/** Every run of ASCII whitespace becomes one space, then the ends are trimmed. */
export function collapseAsciiSpace(s) {
  let out = '';
  let pendingSpace = false;
  for (const ch of String(s)) {
    if (isAsciiSpace(ch)) {
      pendingSpace = out !== '';
      continue;
    }
    if (pendingSpace) out += ' ';
    pendingSpace = false;
    out += ch;
  }
  return out;
}

function scalarLength(s) {
  return Array.from(s).length;
}

/** Order by Unicode code point — Rust's order on UTF-8 bytes. */
function compareCodePoints(a, b) {
  const x = Array.from(a);
  const y = Array.from(b);
  const n = Math.min(x.length, y.length);
  for (let i = 0; i < n; i++) {
    const d = x[i].codePointAt(0) - y[i].codePointAt(0);
    if (d !== 0) return d < 0 ? -1 : 1;
  }
  return x.length === y.length ? 0 : (x.length < y.length ? -1 : 1);
}

/** The Agent Skills name grammar: lowercase ASCII words joined by single hyphens. */
export function isValidSkillName(name) {
  return typeof name === 'string'
    && name.length > 0
    && name.length <= SKILL_NAME_MAX_CHARS
    && /^[a-z0-9]+(-[a-z0-9]+)*$/.test(name);
}

function isSkillNode(node) {
  return !!node && typeof node === 'object' && node.node_type === SKILL_NODE_TYPE;
}

function propsOf(node) {
  return (node && node.properties && typeof node.properties === 'object') ? node.properties : {};
}

/* A skill that may be offered: see the header. */
function usable(node) {
  if (!isSkillNode(node)) return false;
  const props = propsOf(node);
  if (props.enabled === false) return false;
  if (!isValidSkillName(props.name)) return false;
  return typeof props.description === 'string' && collapseAsciiSpace(props.description) !== '';
}

/* A local skill that switches a package skill of its name OFF. */
function masks(node) {
  if (!isSkillNode(node)) return false;
  const props = propsOf(node);
  return props.enabled === false && isValidSkillName(props.name);
}

function byNameThenPath(a, b) {
  const byName = compareCodePoints(propsOf(a).name, propsOf(b).name);
  if (byName !== 0) return byName;
  return compareCodePoints(String(a.path || ''), String(b.path || ''));
}

/**
 * The resolved skill set, in index order. PURE.
 *
 * @param {{
 *   declared?: Array<{source: 'agent'|'step', workspace: string, path: string, node: object|null}>,
 *   installation?: object[],   // children of functions:/local/skills
 *   pkg?: object[],            // children of functions:/skills
 *   globalsEnabled?: boolean,
 * }} input
 * @returns {Array<{name: string, description: string, workspace: string, path: string, source: string}>}
 */
export function selectSkills({ declared, installation, pkg, globalsEnabled } = {}) {
  const out = [];
  const seen = new Set();
  const take = (node, workspace, path, source) => {
    if (!usable(node)) return;
    const props = propsOf(node);
    if (seen.has(props.name)) return;
    seen.add(props.name);
    out.push({ name: props.name, description: props.description, workspace, path, source });
  };

  for (const d of Array.isArray(declared) ? declared : []) {
    if (!d) continue;
    const source = d.source === 'step' ? 'step' : 'agent';
    take(d.node, d.workspace || 'functions', String(d.path || ''), source);
  }

  if (globalsEnabled !== false) {
    /* Only a skill with a valid name can be offered or mask anything, so the
     * sort never sees anything else — which keeps it total and ASCII-ordered. */
    const named = (n) => isSkillNode(n) && isValidSkillName(propsOf(n).name);
    const local = (Array.isArray(installation) ? installation : []).filter(named).sort(byNameThenPath);
    const shipped = (Array.isArray(pkg) ? pkg : []).filter(named).sort(byNameThenPath);
    const masked = new Set(local.filter(masks).map((n) => propsOf(n).name));
    for (const node of local) {
      take(node, GLOBAL_SKILL_ROOTS.installation.workspace, String(node.path || ''), 'installation');
    }
    for (const node of shipped) {
      if (masked.has(propsOf(node).name)) continue;
      take(node, GLOBAL_SKILL_ROOTS.package.workspace, String(node.path || ''), 'package');
    }
  }
  return out;
}

/** One description as the index shows it: collapsed, and cut at 200 with an ellipsis. */
export function skillIndexDescription(description) {
  const chars = Array.from(collapseAsciiSpace(description == null ? '' : description));
  if (chars.length <= SKILL_DESCRIPTION_MAX_CHARS) return chars.join('');
  return chars.slice(0, SKILL_DESCRIPTION_MAX_CHARS - 1).join('') + '…';
}

/* How many skills fit, and the block they make — the single place the cap lives. */
function indexLayout(skills) {
  const list = Array.isArray(skills) ? skills : [];
  if (list.length === 0) return { text: '', listed: 0, total: 0 };
  let text = INDEX_HEADER;
  let chars = scalarLength(INDEX_HEADER);
  let listed = 0;
  for (const skill of list) {
    if (listed >= SKILL_INDEX_MAX_LINES) break;
    const line = '\n- ' + skill.name + ' — ' + skillIndexDescription(skill.description);
    const lineChars = scalarLength(line);
    if (chars + lineChars > SKILL_INDEX_MAX_CHARS) break;
    text += line;
    chars += lineChars;
    listed++;
  }
  if (listed < list.length) {
    text += '\n[index truncated: ' + (list.length - listed) + ' more skills not listed]';
  }
  return { text, listed, total: list.length };
}

/** The `## Skills` block. Empty set, empty string. */
export function skillIndexText(skills) {
  return indexLayout(skills).text;
}

/** How many resolved skills the index had to leave out — for logging. */
export function skillIndexOmitted(skills) {
  const layout = indexLayout(skills);
  return layout.total - layout.listed;
}

/**
 * EVERYTHING appended after the base prompt, planning addition and memory: the
 * skills index, then the agent's `## Rules`, which stays last as the strongest
 * statement. The only place either heading is written.
 */
export function composeInstructionTail({ skills, rules } = {}) {
  let tail = skillIndexText(skills);
  if (Array.isArray(rules)) {
    const items = rules.filter((r) => typeof r === 'string');
    if (items.length > 0) {
      tail += '\n\n## Rules\n' + items.map((r) => '- ' + r).join('\n');
    }
  }
  return tail;
}

/** Append the tail. An empty tail returns the prompt AS GIVEN, undefined included. */
export function appendTail(systemPrompt, tail) {
  if (tail === '') return systemPrompt;
  return systemPrompt ? systemPrompt + tail : tail;
}

/* A reference as `tools:` stores it: {raisin:ref, raisin:workspace}, and after a
 * server write also raisin:path. A bare '/path' string is accepted too. */
function refTarget(ref) {
  if (!ref) return null;
  if (typeof ref === 'string') {
    return ref.startsWith('/') ? { workspace: 'functions', path: ref, id: null } : null;
  }
  if (typeof ref !== 'object') return null;
  const workspace = (typeof ref['raisin:workspace'] === 'string' && ref['raisin:workspace'])
    || (typeof ref.workspace === 'string' && ref.workspace)
    || 'functions';
  const rawRef = typeof ref['raisin:ref'] === 'string' ? ref['raisin:ref'] : '';
  const path = (typeof ref['raisin:path'] === 'string' && ref['raisin:path'])
    || (rawRef.startsWith('/') ? rawRef : '')
    || (typeof ref.target === 'string' && ref.target.startsWith('/') ? ref.target : '');
  if (path) return { workspace, path, id: null };
  if (rawRef) return { workspace, path: null, id: rawRef };
  return null;
}

/**
 * The IO half: read what the agent (and a step) references and the two global
 * folders, then select. Reads run in parallel; a read that throws counts as
 * absent and is reported through `onReadError` — an unreadable skill must not
 * cost the agent its turn. `getNodeById` is optional and serves a reference that
 * carries only an id.
 *
 * @returns {Promise<Array<{name, description, workspace, path, source}>>}
 */
export async function loadSkillGrant({ agentProps, stepSkills, getNode, getNodeById, getChildren, onReadError } = {}) {
  const props = agentProps || {};
  const guard = (what, fn) => Promise.resolve()
    .then(fn)
    .catch((error) => {
      if (typeof onReadError === 'function') onReadError({ ...what, error });
      return null;
    });

  const refs = [];
  for (const ref of Array.isArray(props.skills) ? props.skills : []) refs.push({ source: 'agent', ref });
  for (const ref of Array.isArray(stepSkills) ? stepSkills : []) refs.push({ source: 'step', ref });

  const declaredReads = refs.map(({ source, ref }) => {
    const target = refTarget(ref);
    if (!target) return Promise.resolve(null);
    if (target.path) {
      return guard({ workspace: target.workspace, path: target.path }, () => getNode(target.workspace, target.path))
        .then((node) => ({ source, workspace: target.workspace, path: target.path, node: node || null }));
    }
    if (typeof getNodeById !== 'function') return Promise.resolve(null);
    return guard({ workspace: target.workspace, id: target.id }, () => getNodeById(target.workspace, target.id))
      .then((node) => (node && typeof node.path === 'string'
        ? { source, workspace: target.workspace, path: node.path, node }
        : null));
  });

  const globalsEnabled = props.global_skills !== false;
  const list = (root) => (globalsEnabled && typeof getChildren === 'function'
    ? guard({ workspace: root.workspace, path: root.path }, () => getChildren(root.workspace, root.path))
    : Promise.resolve([]));

  const [declared, installation, pkg] = await Promise.all([
    Promise.all(declaredReads),
    list(GLOBAL_SKILL_ROOTS.installation),
    list(GLOBAL_SKILL_ROOTS.package),
  ]);

  return selectSkills({
    declared: declared.filter(Boolean),
    installation: Array.isArray(installation) ? installation : [],
    pkg: Array.isArray(pkg) ? pkg : [],
    globalsEnabled,
  });
}

/** The grant as load-skill and the flow runtime carry it: names and places, no text. */
export function grantEntries(skills) {
  return (Array.isArray(skills) ? skills : []).map((s) => ({ name: s.name, workspace: s.workspace, path: s.path }));
}

/** A skill body as load-skill returns it: capped at 20000 scalar values, and says so. */
export function skillBodyText(body) {
  const text = typeof body === 'string' ? body : '';
  const chars = Array.from(text);
  if (chars.length <= SKILL_BODY_MAX_CHARS) return text;
  return chars.slice(0, SKILL_BODY_MAX_CHARS).join('')
    + '\n[truncated: first ' + SKILL_BODY_MAX_CHARS + ' of ' + chars.length + ' characters shown]';
}
