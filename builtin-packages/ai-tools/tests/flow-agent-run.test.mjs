/**
 * A flow's agent step starts a durable AgentRun: the conversation holds the
 * step's prompt, the run carries a WAITER naming the flow instance (from the
 * runtime's own stamp), and the step's response_format reaches the model turn
 * as an output schema.
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRaisin } from './support/fake-raisin.mjs';

const fake = fakeRaisin();
const { handler, outputSchemaOf, flowChatName } = await import('../content/functions/lib/raisin/ai/flow-agent-run/index.js');
const { initState } = await import('../content/functions/lib/raisin/ai/agent-run-reducer/state.js');

fake.put('functions', '/agents/classifier', { node_type: 'raisin:AIAgent', properties: { title: 'Classifier', tools: [] } });

const flow = { instance_id: 'inst-1', step_id: 'classify' };

test('a step becomes a conversation and a run that the flow waits for', async () => {
  const out = await handler({
    agent_ref: '/agents/classifier', prompt: 'Classify: great', visit: 2, max_model_calls: 4,
    response_format: { type: 'json_schema', json_schema: { name: 'x', schema: { type: 'object' } } },
    __raisin_flow: flow,
  });
  const chat = `/agents/classifier/inbox/chats/${flowChatName(flow, 2)}`;
  assert.equal(out.chat_path, chat);
  const brief = (await raisin.nodes.get('ai', `${chat}/prompt`)).properties;
  assert.equal(brief.content, 'Classify: great');
  // raisin:Message requires body, sender_id, message_type and status.
  for (const k of ['body', 'sender_id', 'message_type', 'status']) assert.ok(brief[k], `prompt message carries ${k}`);
  // raisin:Conversation requires participants; only the agent (a non-agent
  // participant would be read as the human a reply goes to).
  assert.deepEqual((await raisin.nodes.get('ai', chat)).properties.participants, ['agent:classifier']);
  const req = fake.calls.creates.at(-1);
  assert.deepEqual(req.waiter, { kind: 'flow_instance', target: 'inst-1', branch: '', data: { step_id: 'classify' } });
  assert.equal(req.create_key, 'flow:inst-1:classify:2');
  assert.equal(req.budgets.max_model_calls, 4);
  assert.equal(req.budgets.on_exceeded, 'fail');
  assert.deepEqual(req.input.config.output_schema, { type: 'object' });
  assert.equal(req.input.config.execution_mode, 'automatic');
  // The reducer carries the schema to every model turn.
  assert.deepEqual(initState(req.input, 1).cfg.output_schema, { type: 'object' });
});

test('only the runtime can start one', async () => {
  await assert.rejects(() => handler({ agent_ref: '/agents/classifier', prompt: 'x' }), /called by a flow agent step/);
  await assert.rejects(() => handler({ agent_ref: '/agents/none', __raisin_flow: flow }), /not an installed agent/);
});

test('response formats map to a schema', () => {
  assert.deepEqual(outputSchemaOf({ type: 'json_object' }), { type: 'object' });
  assert.deepEqual(outputSchemaOf({ schema: { type: 'string' } }), { type: 'string' });
  assert.equal(outputSchemaOf(null), null);
});
