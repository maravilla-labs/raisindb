/**
 * ABSENT MEANS ABSENT: with no skills, no memory and no rules, a model turn's
 * system prompt is the agent's own prompt, byte for byte. Memory is the run
 * OWNER's (the agent the run executes and the user it acts for, from the run
 * record), the skill index sits before `## Rules`, and the reducer's step
 * instructions and run facts come after.
 *
 * Run: node --test builtin-packages/ai-tools/tests/agent-prompt-absent.test.mjs
 */
import assert from 'node:assert/strict';
import test from 'node:test';

import { buildSystemPrompt } from '../content/functions/lib/raisin/ai/agent-run-model-turn/context.js';
import { formatMemoryForPrompt, memoryOwnerOf, memoryPath } from '../content/functions/lib/raisin/ai/agent-shared/memory.js';
import { appendTail, composeInstructionTail } from '../content/functions/lib/raisin/ai/agent-shared/skills.js';

function installMemory(store) {
  const reads = [];
  globalThis.raisin = {
    nodes: {
      async get(ws, path) {
        reads.push(`${ws}:${path}`);
        return store[`${ws}:${path}`] ?? null;
      },
    },
  };
  return reads;
}

const OWNER = { agentName: 'probe', userId: 'user-1' };
const base = (extra = {}) => ({ agentProps: { system_prompt: 'You are a probe.', ...extra }, request: {}, skills: [], definitions: [] });

test('no skills, no memory, no rules: the agent prompt, byte for byte', async () => {
  installMemory({});
  assert.equal(await buildSystemPrompt({ ...base(), memoryOwner: OWNER }), 'You are a probe.');
  assert.equal(await buildSystemPrompt({ ...base(), memoryOwner: null }), 'You are a probe.');
});

test('the run owner\'s memory, then the rules', async () => {
  const reads = installMemory({ [`ai:${memoryPath('probe', 'user-1')}`]: { properties: { content: 'Likes otters.' } } });
  const got = await buildSystemPrompt({ ...base({ rules: ['End with OVER.'] }), memoryOwner: OWNER });
  assert.equal(got, `You are a probe.${formatMemoryForPrompt('Likes otters.')}\n\n## Rules\n- End with OVER.`);
  assert.deepEqual(reads, ['ai:/agents/probe/memory/user-1']);
});

test('the memory owner comes from the run record: the agent it runs and the user it acts for', () => {
  assert.deepEqual(memoryOwnerOf({
    agent_ref: 'functions:/agents/probe', principal: { kind: 'agent', id: 'functions:/agents/probe', on_behalf_of: 'user-1' },
  }), OWNER);
  assert.deepEqual(memoryOwnerOf({ agent_ref: '/agents/probe', principal: { kind: 'user', id: 'user-9' } }), { agentName: 'probe', userId: 'user-9' });
  assert.equal(memoryOwnerOf({ agent_ref: 'functions:/agents/probe', principal: { kind: 'agent', id: 'x' } }), null);
  assert.equal(memoryOwnerOf({ agent_ref: 'functions:/lib/not-an-agent', principal: { kind: 'user', id: 'u' } }), null);
});

test('step instructions and run facts follow the agent\'s own prompt', async () => {
  installMemory({});
  const got = await buildSystemPrompt({
    ...base(),
    request: { instructions: 'Stop here.', context: { facts: { objective: 'Fix typos' } } },
    memoryOwner: null,
  });
  assert.ok(got.startsWith('You are a probe.\n\n## Instructions for this step\nStop here.'));
  assert.match(got, /## Run state \(authoritative, from the runtime\)\nObjective: Fix typos/);
});

test('appendTail + composeInstructionTail reproduce the rules block exactly', () => {
  for (const b of ['', 'You are X.', 'multi\nline\n\n']) {
    for (const rules of [undefined, null, [], ['a'], ['a', 'b c'], 'nope']) {
      const want = Array.isArray(rules) && rules.length ? `${b}\n\n## Rules\n${rules.map((r) => `- ${r}`).join('\n')}` : b;
      assert.equal(appendTail(b, composeInstructionTail({ rules, skills: [] })), want, JSON.stringify({ b, rules }));
    }
  }
});
