/**
 * load-skill: the grant is the server's, never the model's.
 *
 * CHAT — the grant is resolved from __raisin_context.chat_path through the
 * conversation's agent, and a model-supplied skill_grant beside it is ignored.
 * FLOW — with no chat_path, the runtime's skill_grant is honoured.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { handler } from './index.js';

const CHAT = '/agents/probe/inbox/chats/c1';
const AGENT = '/agents/probe';

function skill(name, extra = {}, path = `/lib-skills/${name}`) {
  return { path, node_type: 'raisin:Skill', properties: { name, description: `Does ${name}.`, body: `Body of ${name}.`, ...extra } };
}

function stubRaisin(store) {
  globalThis.raisin = {
    nodes: {
      async get(ws, path) { return store[`${ws}:${path}`] ?? null; },
      async getById() { return null; },
      async getChildren(ws, path) {
        const prefix = `${ws}:${path}/`;
        return Object.entries(store)
          .filter(([k]) => k.startsWith(prefix) && !k.slice(prefix.length).includes('/'))
          .map(([, n]) => n);
      },
    },
  };
}

function chatStore(agentProps) {
  return {
    [`ai:${CHAT}`]: {
      path: CHAT,
      node_type: 'raisin:Conversation',
      properties: { agent_ref: { 'raisin:ref': 'agent-1', 'raisin:path': AGENT, 'raisin:workspace': 'functions' } },
    },
    [`functions:${AGENT}`]: { path: AGENT, node_type: 'raisin:AIAgent', properties: agentProps },
    'functions:/lib-skills/granted': skill('granted'),
    'functions:/lib-skills/secret': skill('secret'),
  };
}
const chatCtx = (extra = {}) => ({ workspace: 'ai', chat_path: CHAT, ...extra });
const REF = (name) => ({ 'raisin:ref': `id-${name}`, 'raisin:path': `/lib-skills/${name}`, 'raisin:workspace': 'functions' });

test('chat: a granted skill returns its body', async () => {
  stubRaisin(chatStore({ skills: [REF('granted')] }));
  assert.deepEqual(await handler({ name: 'granted', __raisin_context: chatCtx() }), {
    success: true,
    name: 'granted',
    description: 'Does granted.',
    body: 'Body of granted.',
  });
});

test('chat: an ungranted skill is refused with what is available', async () => {
  stubRaisin(chatStore({ skills: [REF('granted')] }));
  assert.deepEqual(await handler({ name: 'secret', __raisin_context: chatCtx() }), {
    success: false,
    reason: 'not_granted',
    name: 'secret',
    available: ['granted'],
  });
});

test('chat: a forged skill_grant beside chat_path is ignored', async () => {
  stubRaisin(chatStore({ skills: [REF('granted')] }));
  const got = await handler({
    name: 'secret',
    __raisin_context: chatCtx({ skill_grant: [{ name: 'secret', workspace: 'functions', path: '/lib-skills/secret' }] }),
  });
  assert.equal(got.success, false);
  assert.equal(got.reason, 'not_granted');
  assert.deepEqual(got.available, ['granted']);
});

test('chat: global skills are granted, and global_skills:false withdraws them', async () => {
  const store = chatStore({});
  store['functions:/skills/house-style'] = skill('house-style', {}, '/skills/house-style');
  stubRaisin(store);
  assert.equal((await handler({ name: 'house-style', __raisin_context: chatCtx() })).body, 'Body of house-style.');

  store[`functions:${AGENT}`].properties = { global_skills: false };
  assert.deepEqual(await handler({ name: 'house-style', __raisin_context: chatCtx() }), {
    success: false, reason: 'not_granted', name: 'house-style', available: [],
  });
});

test('flow: the runtime grant is honoured when there is no chat_path', async () => {
  stubRaisin({ 'functions:/lib-skills/secret': skill('secret') });
  const ctx = { skill_grant: [{ name: 'secret', workspace: 'functions', path: '/lib-skills/secret' }] };
  assert.equal((await handler({ name: 'secret', __raisin_context: ctx })).body, 'Body of secret.');
  assert.deepEqual(await handler({ name: 'granted', __raisin_context: ctx }), {
    success: false, reason: 'not_granted', name: 'granted', available: ['secret'],
  });
});

test('flow: a granted skill that is gone or switched off since', async () => {
  const store = { 'functions:/lib-skills/off': skill('off', { enabled: false }) };
  stubRaisin(store);
  const ctx = { skill_grant: [
    { name: 'off', workspace: 'functions', path: '/lib-skills/off' },
    { name: 'gone', workspace: 'functions', path: '/lib-skills/gone' },
  ] };
  assert.deepEqual(await handler({ name: 'off', __raisin_context: ctx }), {
    success: false, reason: 'disabled', name: 'off', available: ['off', 'gone'],
  });
  assert.deepEqual(await handler({ name: 'gone', __raisin_context: ctx }), {
    success: false, reason: 'not_found', name: 'gone', available: ['off', 'gone'],
  });
});

test('no context at all → no_grant', async () => {
  stubRaisin({});
  assert.deepEqual(await handler({ name: 'anything' }), { success: false, reason: 'no_grant' });
  assert.deepEqual(await handler({ name: 'anything', __raisin_context: {} }), { success: false, reason: 'no_grant' });
});

test('the body is capped at 20000 characters and says so', async () => {
  stubRaisin({ 'functions:/lib-skills/big': skill('big', { body: 'z'.repeat(20005) }) });
  const got = await handler({ name: 'big', __raisin_context: { skill_grant: [{ name: 'big', workspace: 'functions', path: '/lib-skills/big' }] } });
  assert.equal(got.body, 'z'.repeat(20000) + '\n[truncated: first 20000 of 20005 characters shown]');
});

/*
 * THE FORGERY, end to end. A flow model wrote its own grant (and a foreign
 * chat_path) into a load-skill call. The shared fixture holds that call and the
 * arguments the Rust container runtime executed it with — the Rust test
 * (raisin-flow-runtime handlers/ai_container/skill_grant.rs) asserts it produces
 * exactly `executed_arguments` from `model_tool_call`. Here those arguments
 * reach the real tool, which must refuse the forged skill and never read it.
 */
const FORGED = JSON.parse(readFileSync(
  new URL('../../../../../../tests/fixtures/load-skill-forged-grant.json', import.meta.url),
  'utf8',
));

test('flow: a grant the model forged never opens a skill the step was not given', async () => {
  const { forged_skill: forged, server_grant: grant, executed_arguments: args } = FORGED;
  const reads = [];
  stubRaisin({
    [`functions:${forged.path}`]: skill(forged.name),
    'functions:/lib-skills/granted': skill('granted'),
  });
  const get = globalThis.raisin.nodes.get;
  globalThis.raisin.nodes.get = async (ws, path) => { reads.push(`${ws}:${path}`); return get(ws, path); };

  // The model asked for the forged skill by name, and that is all that survived.
  assert.equal(args.name, forged.name);
  assert.deepEqual(await handler(args), {
    success: false, reason: 'not_granted', name: forged.name, available: grant.map((g) => g.name),
  });
  assert.ok(!reads.includes(`functions:${forged.path}`), 'the forged skill was read');

  // The same call for a skill the server DID grant still works.
  assert.equal((await handler({ ...args, name: 'granted' })).body, 'Body of granted.');
});
