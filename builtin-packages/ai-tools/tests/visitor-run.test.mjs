/**
 * A run made for an anonymous VISITOR (`visitor:<key>`): the conversation's
 * token budget caps the run and fails it instead of pausing (a paused run is
 * resumed, with raised budgets, by the next message), no node-development
 * grant, and no tools that keep per-person state or act as the system.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from './support/fake-raisin.mjs';

fakeRaisin();
const {
  buildCreateRequest, visitorBudgets, isVisitor, VISITOR_EXCLUDED_TOOLS, routeToAgentRun,
} = await import('../content/functions/lib/raisin/ai/agent-shared/run-entry.js');

const VISITOR = 'visitor:0123456789abcdef0123456789abcdef';
const base = {
  workspace: 'ai', chatPath: '/agents/site/inbox/chats/vchat-1',
  chat: { id: 'chat-1', properties: { total_tokens_used: 4000 } },
  message: { id: 'm1', path: '/agents/site/inbox/chats/vchat-1/msg-1', properties: { content: 'hi' } },
  agentRef: 'functions:/agents/site', tools: [], plan: null,
};

test('visitor ids are recognised', () => {
  assert.equal(isVisitor(VISITOR), true);
  assert.equal(isVisitor('user-1'), false);
  assert.equal(isVisitor(null), false);
});

test('a visitor run is capped by what is left of the conversation budget, and fails when it runs out', () => {
  const agentProps = {
    anonymous: { enabled: true, max_conversation_tokens: 10000 },
    node_dev: { roots: [{ workspace: 'content' }] },
  };
  const req = buildCreateRequest({ ...base, agentProps, senderId: VISITOR, actingUser: VISITOR });
  assert.equal(req.budgets.max_total_tokens, 6000, '10000 budget − 4000 used');
  assert.equal(req.budgets.on_exceeded, 'fail', 'a visitor run never pauses for a resume');
  assert.equal(req.executor_config.node_dev, undefined, 'no working grant for the open internet');
  assert.equal(req.on_behalf_of, VISITOR, 'core resolves the anonymous tool grant from this');
});

test('a user run keeps the ordinary budgets', () => {
  const agentProps = { anonymous: { enabled: true, max_conversation_tokens: 10000 } };
  const req = buildCreateRequest({ ...base, agentProps, senderId: 'user-1', actingUser: 'user-1' });
  assert.equal(req.budgets.max_total_tokens, undefined);
  assert.equal(req.budgets.on_exceeded, 'pause');
});

test('an exhausted budget still leaves a positive cap (core refuses the first call)', () => {
  const b = visitorBudgets({ max_model_calls: 60 }, { anonymous: { max_conversation_tokens: 100 } },
    { properties: { total_tokens_used: 500 } });
  assert.equal(b.max_total_tokens, 1);
});

test('a visitor run is not offered memory or delegation tools', async () => {
  const { api, put, calls } = fakeRaisin();
  for (const path of ['/lib/raisin/ai/remember', '/lib/site/search']) {
    put('functions', path, {
      node_type: 'raisin:Function',
      properties: { name: path.split('/').pop(), description: 'd', input_schema: { type: 'object', properties: { q: { type: 'string' } } } },
    });
  }
  const chatPath = '/agents/site/inbox/chats/vchat-2';
  const chat = put('ai', chatPath, { node_type: 'raisin:Conversation', properties: { human_sender_id: VISITOR, agent_ref: { 'raisin:path': '/agents/site' } } });
  const message = put('ai', `${chatPath}/msg-1`, { node_type: 'raisin:Message', properties: { role: 'user', content: 'hi', sender_id: VISITOR } });
  await routeToAgentRun({
    workspace: 'ai', chatPath, chat, message,
    agentProps: {
      anonymous: { enabled: true },
      tools: [
        { 'raisin:path': '/lib/raisin/ai/remember', 'raisin:workspace': 'functions' },
        { 'raisin:path': '/lib/site/search', 'raisin:workspace': 'functions' },
      ],
    },
    agentPath: '/agents/site', agentWorkspace: 'functions',
    outboxCtx: { senderId: VISITOR }, streamChannel: 'chat:vchat-2', hasSkills: false,
  });
  const offered = calls.creates.at(-1).input.tools.map((t) => t.function_path);
  assert.ok(offered.includes('/lib/site/search'), `site tool offered: ${offered}`);
  assert.ok(!offered.some((p) => VISITOR_EXCLUDED_TOOLS.includes(p)), `no memory tools: ${offered}`);
  void api;
});
