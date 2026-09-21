/**
 * ABSENT MEANS ABSENT: with no skills anywhere — none referenced, no globals, or
 * the agent opting out of them — the chat loop's system message is byte for byte
 * what it was before skills existed. With a skill, the index sits before
 * `## Rules`, in both rounds.
 *
 * Both handlers are driven end to end over a mocked `globalThis.raisin` (the
 * pattern of agent-handler/index.test.mjs); the model call captures the
 * messages (and tools) it was given and throws, so nothing after it runs for real.
 */
import assert from 'node:assert/strict';
import test from 'node:test';

import { handleUserMessage } from '../content/functions/lib/raisin/ai/agent-handler/index.js';
import { handleToolResult } from '../content/functions/lib/raisin/ai/agent-continue-handler/index.js';
import { formatMemoryForPrompt } from '../content/functions/lib/raisin/ai/agent-shared/memory.js';
import { appendTail, composeInstructionTail } from '../content/functions/lib/raisin/ai/agent-shared/skills.js';

const AGENT_PATH = '/agents/probe';
const CHAT = '/agents/probe/inbox/chats/c1';
const MSG = `${CHAT}/msg-1`;
const REPLY = `${CHAT}/reply-to-msg-1`;
const RESULT = `${REPLY}/aggregated_result`;
const USER = 'user-1';
const LOAD_SKILL_PATH = '/lib/raisin/ai/load-skill';
const HEADER = '\n\n## Skills\nLoad a skill with the load-skill tool before doing work its description covers; its instructions then apply.';

/** The system prompt exactly as the handlers built it BEFORE this change. */
function preChangeFirstRound({ system_prompt, rules }, memory) {
  let systemPrompt = system_prompt || '';
  if (memory) systemPrompt += formatMemoryForPrompt(memory);
  if (Array.isArray(rules) && rules.length > 0) {
    systemPrompt += '\n\n## Rules\n' + rules.map(r => `- ${r}`).join('\n');
  }
  return systemPrompt;
}
function preChangeContinuation({ system_prompt, rules }, memory) {
  let systemPrompt = system_prompt;
  if (memory) {
    const block = formatMemoryForPrompt(memory);
    systemPrompt = systemPrompt ? systemPrompt + block : block;
  }
  if (Array.isArray(rules) && rules.length > 0) {
    const rulesBlock = '\n\n## Rules\n' + rules.map(r => `- ${r}`).join('\n');
    systemPrompt = systemPrompt ? systemPrompt + rulesBlock : rulesBlock;
  }
  return systemPrompt;
}

/**
 * A mocked runtime. `store` is keyed 'workspace:path'; getChildren answers the
 * store's direct children of a path, so /skills and /local/skills are [] unless
 * a test puts a skill there. Returns the messages and tools handed to the model
 * on its first call.
 */
function installRaisin(store, reads) {
  const captured = { messages: null, tools: null };
  globalThis.raisin = {
    nodes: {
      async get(ws, path) {
        if (reads) reads.push(`${ws}:${path}`);
        return store[`${ws}:${path}`] ?? null;
      },
      async updateProperty() {},
      async update() {},
      async create(ws, parent, body) {
        return { path: `${parent}/${body?.name || 'x'}`, node_type: body?.node_type, properties: body?.properties || {} };
      },
      async getChildren(ws, path) {
        if (reads) reads.push(`children ${ws}:${path}`);
        const prefix = `${ws}:${path}/`;
        return Object.entries(store)
          .filter(([k]) => k.startsWith(prefix) && !k.slice(prefix.length).includes('/'))
          .map(([, node]) => node);
      },
    },
    sql: { async query() { return []; } },
    events: { async emit() {} },
    ai: {
      async completion(req) {
        if (!captured.messages) {
          captured.messages = req.messages;
          captured.tools = req.tools;
        }
        throw new Error('captured-model-call');
      },
    },
  };
  return captured;
}

function baseStore(agentProps, { memory } = {}) {
  const store = {
    [`ai:${MSG}`]: {
      path: MSG,
      node_type: 'raisin:Message',
      properties: { role: 'user', message_type: 'chat', status: 'delivered', content: 'hello' },
    },
    [`ai:${CHAT}`]: {
      path: CHAT,
      node_type: 'raisin:Conversation',
      properties: {
        agent_ref: { 'raisin:path': AGENT_PATH, 'raisin:workspace': 'functions' },
        participants: [USER],
        human_sender_id: USER,
        human_sender_path: `/users/${USER}`,
      },
    },
    [`ai:${AGENT_PATH}`]: {
      path: AGENT_PATH,
      node_type: 'raisin:Agent',
      properties: { user_id: 'agent:probe', display_name: 'Probe' },
    },
    [`functions:${AGENT_PATH}`]: {
      path: AGENT_PATH,
      node_type: 'raisin:AIAgent',
      properties: { provider: 'groq', model: 'llama', ...agentProps },
    },
    [`functions:${LOAD_SKILL_PATH}`]: {
      id: 'fn-load-skill',
      name: 'load-skill',
      path: LOAD_SKILL_PATH,
      node_type: 'raisin:Function',
      properties: { description: 'Load a skill.', execution_mode: 'inline', category: 'skills', input_schema: { type: 'object', properties: { name: { type: 'string' } } } },
    },
  };
  if (memory) {
    store[`ai:${AGENT_PATH}/memory/${USER}`] = { path: 'm', node_type: 'raisin:AgentUserContext', properties: { content: memory } };
  }
  return store;
}

async function firstRound(store, reads) {
  const captured = installRaisin(store, reads);
  await assert.rejects(() => handleUserMessage({ workspace: 'ai', event: { node_path: MSG } }), /captured-model-call/);
  assert.ok(captured.messages, 'the model was called');
  // The agent's system message is the FIRST entry, when there is one; a later
  // system entry (e.g. "tools are unavailable") is the loop's, not the agent's.
  const first = captured.messages[0];
  return { system: first && first.role === 'system' ? first.content : undefined, tools: captured.tools };
}
async function firstRoundSystem(store, reads) {
  return (await firstRound(store, reads)).system;
}

async function continuation(store) {
  store[`ai:${REPLY}`] = {
    path: REPLY,
    node_type: 'raisin:Message',
    properties: { role: 'assistant', dispatch_phase: 'awaiting_results', finish_reason: 'tool_calls' },
  };
  store[`ai:${RESULT}`] = { path: RESULT, node_type: 'raisin:AIToolResultAggregator', properties: { results: [] } };
  const captured = installRaisin(store);
  try {
    await handleToolResult({ flow_input: { workspace: 'ai', event: { node_path: RESULT } } });
  } catch (e) {
    if (!/captured-model-call/.test(String(e && e.message))) throw e;
  }
  assert.ok(captured.messages, 'the continuation reached the model');
  // The agent's system message is the FIRST entry, when there is one. The loop
  // appends its own "no tools this turn" instruction after the history, and with
  // this mock's empty history that lands first — it is the loop's, not the
  // agent's, so it is not what this test compares.
  const agentEntries = captured.messages.filter(
    (m) => !(m.role === 'system' && /^Tools are unavailable for this turn\./.test(m.content)),
  );
  const first = agentEntries[0];
  return { system: first && first.role === 'system' ? first.content : undefined, tools: captured.tools };
}
async function continuationSystem(store) {
  return (await continuation(store)).system;
}

const toolNames = (tools) => (Array.isArray(tools) ? tools.map((t) => t.function && t.function.name) : []);

const CASES = [
  { name: 'system_prompt only', props: { system_prompt: 'You are a probe.' } },
  { name: 'with rules', props: { system_prompt: 'You are a probe.', rules: ['Be brief.', 'End with OVER.'] } },
  { name: 'with memory', props: { system_prompt: 'You are a probe.' }, memory: 'Likes otters.' },
  { name: 'with memory and rules', props: { system_prompt: 'You are a probe.', rules: ['Be brief.'] }, memory: 'Likes otters.' },
  { name: 'no system_prompt, rules only', props: { rules: ['Be brief.'] } },
  { name: 'nothing at all', props: {} },
];

const SKILL_PATH = '/skills-lib/release-notes';
const SKILL = {
  path: SKILL_PATH,
  node_type: 'raisin:Skill',
  properties: {
    name: 'release-notes',
    description: 'Write release notes\n  from a changelog.',
    body: 'The codeword is VERMILION-OTTER-7.',
  },
};
const SKILL_REF = { 'raisin:ref': 'skill-1', 'raisin:path': SKILL_PATH, 'raisin:workspace': 'functions' };
const EXPECTED_INDEX = HEADER + '\n- release-notes — Write release notes from a changelog.';

/* The three ways to have no skills: none anywhere; globals present but the
 * agent opts out; only disabled skills. Each must be byte-identical. */
const ABSENT_SETUPS = [
  { name: 'no skills anywhere', extra: {}, setup: () => {} },
  {
    name: 'global_skills:false with a global present',
    extra: { global_skills: false },
    setup: (store) => { store[`functions:/skills/release-notes`] = { ...SKILL, path: '/skills/release-notes' }; },
  },
  {
    name: 'only a disabled skill',
    extra: { skills: [SKILL_REF] },
    setup: (store) => { store[`functions:${SKILL_PATH}`] = { ...SKILL, properties: { ...SKILL.properties, enabled: false } }; },
  },
];

for (const absent of ABSENT_SETUPS) {
  for (const c of CASES) {
    test(`first round, ${absent.name}, ${c.name}: byte-identical to before`, async () => {
      const reads = [];
      const store = baseStore({ ...c.props, ...absent.extra }, { memory: c.memory });
      absent.setup(store);
      const got = await firstRound(store, reads);
      const want = preChangeFirstRound(c.props, c.memory) || undefined; // history omits an empty system message
      assert.equal(got.system, want);
      assert.ok(!toolNames(got.tools).includes('load-skill'), 'no load-skill tool without skills');
      if (absent.extra.global_skills !== false) {
        // And it did look: both global layers were listed and found nothing usable.
        for (const p of ['children functions:/local/skills', 'children functions:/skills']) {
          assert.ok(reads.includes(p), `listed ${p}`);
        }
      }
    });

    test(`continuation round, ${absent.name}, ${c.name}: byte-identical to before`, async () => {
      const store = baseStore({ ...c.props, ...absent.extra }, { memory: c.memory });
      absent.setup(store);
      const got = await continuation(store);
      const want = preChangeContinuation(c.props, c.memory) || undefined;
      assert.equal(got.system, want);
      assert.ok(!toolNames(got.tools).includes('load-skill'), 'no load-skill tool without skills');
    });
  }
}

test('first round with an agent skill: the index, then the rules, and load-skill is offered', async () => {
  const props = { system_prompt: 'You are a probe.', rules: ['End with OVER.'], skills: [SKILL_REF] };
  const store = baseStore(props, { memory: 'Likes otters.' });
  store[`functions:${SKILL_PATH}`] = SKILL;
  const got = await firstRound(store);
  assert.equal(got.system, 'You are a probe.' + formatMemoryForPrompt('Likes otters.') + EXPECTED_INDEX + '\n\n## Rules\n- End with OVER.');
  assert.ok(!got.system.includes('VERMILION'), 'the body never goes in the prompt');
  assert.deepEqual(toolNames(got.tools).filter((n) => n === 'load-skill'), ['load-skill']);
});

test('continuation round with an agent skill: the same tail, and load-skill is offered', async () => {
  const props = { system_prompt: 'You are a probe.', rules: ['End with OVER.'], skills: [SKILL_REF] };
  const store = baseStore(props);
  store[`functions:${SKILL_PATH}`] = SKILL;
  const got = await continuation(store);
  assert.equal(got.system, 'You are a probe.' + EXPECTED_INDEX + '\n\n## Rules\n- End with OVER.');
  assert.deepEqual(toolNames(got.tools).filter((n) => n === 'load-skill'), ['load-skill']);
});

test('a global skill reaches an agent that lists none', async () => {
  const props = { system_prompt: 'You are a probe.' };
  const store = baseStore(props);
  store[`functions:/skills/release-notes`] = { ...SKILL, path: '/skills/release-notes' };
  assert.equal(await firstRoundSystem(store), 'You are a probe.' + EXPECTED_INDEX);
});

test('an agent that already lists load-skill is not offered it twice', async () => {
  const props = {
    system_prompt: 'You are a probe.',
    skills: [SKILL_REF],
    tools: [{ 'raisin:ref': 'fn-load-skill', 'raisin:path': LOAD_SKILL_PATH, 'raisin:workspace': 'functions' }],
  };
  const store = baseStore(props);
  store[`functions:${SKILL_PATH}`] = SKILL;
  const got = await firstRound(store);
  assert.deepEqual(toolNames(got.tools).filter((n) => n === 'load-skill'), ['load-skill']);
});

test('appendTail + composeInstructionTail reproduce the old rules block exactly', () => {
  for (const base of ['', undefined, 'You are X.', 'multi\nline\n\n']) {
    for (const rules of [undefined, null, [], ['a'], ['a', 'b c'], 'nope']) {
      const want = preChangeContinuation({ system_prompt: base, rules });
      assert.equal(appendTail(base, composeInstructionTail({ rules, skills: [] })), want, JSON.stringify({ base, rules }));
    }
  }
});
