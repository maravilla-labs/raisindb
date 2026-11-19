/**
 * Real reducer transcripts of a run with client tools, for the Rust contract
 * decoder: `crates/raisin-agent-contract/fixtures/js-reducer/client-tools.json`.
 *
 * Each entry is `{scenario, request, response}` produced by the JS reducer
 * itself; the Rust test `js_reducer` decodes EVERY response as a
 * `ReducerResponse` (the exact decode core does in `agent_reducer.rs`) and runs
 * `validate_response`. A shape the Rust types do not know (a tool `kind`, a
 * field) fails there, not in production (v0.7.36: `kind: "client"` in
 * `tools_offered` was `response_undecodable`).
 *
 * Regenerate: node builtin-packages/ai-tools/tests/client-tools-fixtures.mjs --write
 * Checked for drift by client-tools-fixtures.test.mjs.
 */
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { handler } from '../content/functions/lib/raisin/ai/agent-run-reducer/index.js';

const CHAT = '/agents/a/inbox/chats/c';
export const FIXTURE_PATH = fileURLToPath(new URL('../../../crates/raisin-agent-contract/fixtures/js-reducer/client-tools.json', import.meta.url));

/** Tools as WebMCP returns them. */
const TOOLS = [
  { name: 'get_document', description: 'Read the open document', inputSchema: { type: 'object', properties: { path: { type: 'string' } } }, annotations: { readOnlyHint: true, untrustedContentHint: true } },
  { name: 'insert_blocks', description: 'Insert blocks', inputSchema: { type: 'object', properties: { blocks: { type: 'array', items: { type: 'object' } } }, required: ['blocks'] }, annotations: { consequentialHint: true } },
];
const POLICY = { kind: 'webmcp', allow: ['*'], deny: [], timeout_ms: 120000 };
const INPUT = {
  text: 'add a hero', message_path: `${CHAT}/m1`,
  tools: [{ name: 'search', kind: 'function', function_path: '/lib/demo/search', mutating: false, replay_safe: true }],
  client_policy: POLICY,
  client: { client_id: 'tab-1', tools: TOOLS, context: { location: { path: '/home' }, document: { title: 'Home' } } },
  attachments: [{ 'raisin:ref': '/uploads/a.png', 'raisin:workspace': 'assets', mime_type: 'image/png', name: 'a.png' }],
  config: { execution_mode: 'automatic' },
  context: { workspace: 'ai', chat_path: CHAT },
};

export async function generate() {
  const log = [];
  const s = { seq: 0, rev: 0, state: null, unanswered: [], open: [] };
  const send = async (scenario, kind, data = {}, extra = {}) => {
    s.seq += 1;
    const request = {
      contract: 'raisin.agent-run.reducer/1', accept: ['raisin.agent-run.reducer/1'],
      run: { run_id: 'run-1', status: 'running', turn: 1, last_seq: s.seq, usage: {}, budgets: {}, open_requests: s.open, unanswered_calls: s.unanswered, subject: { workspace: 'ai', path: CHAT } },
      state: s.state, state_rev: s.rev, event: { seq: s.seq, kind, data, ...extra },
    };
    const response = await handler(JSON.parse(JSON.stringify(request)));
    s.state = response.state;
    s.rev = response.state_rev;
    s.last = response;
    for (const e of response.effects) {
      if (e.kind === 'request_model_turn') s.unanswered = [];
      if (e.kind === 'withdraw_request') s.open = s.open.filter((r) => r.request_id !== e.request_id);
    }
    log.push({ scenario, request: JSON.parse(JSON.stringify(request)), response: JSON.parse(JSON.stringify(response)) });
    return response;
  };
  const op = () => s.last.effects.find((e) => ['call_tool', 'request_model_turn', 'request_approval', 'ask_user'].includes(e.kind));
  const model = (scenario, calls) => {
    const eff = op();
    s.unanswered = calls.map((c) => c.call_id);
    return send(scenario, 'model_turn_completed', { message: { text: '' }, tool_calls: calls, finish_reason: 'tool_calls' }, { effect_id: eff.effect_id, operation_id: `run-1/op/${s.seq}` });
  };
  const park = () => {
    const eff = op();
    s.open = [{ request_id: 'run-1/req/1', kind: 'external', effect_id: eff.effect_id, expires_at_ms: 1795089999999 }];
    return eff;
  };

  // 1. RunStarted with client tools present.
  await send('run_started_with_client_tools', 'run_started', { input: INPUT });
  // 2. The model calls a page tool: dispatched to the relay.
  await model('page_call_dispatch', [{ call_id: 'c1', name: 'page.insert_blocks', args: { blocks: [{ type: 'hero' }] } }]);
  // 3. The page answers (new tool set and context ride the envelope).
  let eff = park();
  s.open = [];
  await send('client_result_applied', 'tool_result', {
    call_id: eff.for_call_id, tool: eff.tool,
    envelope: {
      envelope: 'raisin.tool-result/1', operation_id: 'client:c1', source: 'client', status: 'succeeded',
      payload: { ok: true, content: { inserted: 1 } }, untrusted: true,
      client_tools: [TOOLS[0]], client_context: { document: { title: 'Home', blocks: 3 } },
    },
  }, { effect_id: eff.effect_id, operation_id: 'run-1/op/90' });
  // 4. A stale page tool is answered at once.
  await model('stale_page_tool', [{ call_id: 'c2', name: 'page.vanished', args: {} }]);
  // 5. Another call that the page never answers: the deadline closes it.
  await model('page_call_dispatch_read_only', [{ call_id: 'c3', name: 'page__get_document', args: {} }]);
  park();
  s.open = [];
  await send('client_call_expired', 'request_closed', { request_id: 'run-1/req/1', reason: 'expired' });
  // 6. A call parked, then the user steers: the wait is withdrawn and answered.
  await model('page_call_before_steer', [{ call_id: 'c4', name: 'page.insert_blocks', args: { blocks: [] } }]);
  park();
  await send('steer_with_client', 'user_input', {
    input: { text: 'never mind', message_path: `${CHAT}/m2`, client_policy: POLICY, client: { client_id: 'tab-1', tools: [TOOLS[1]], context: { location: { path: '/about' } } } },
  });
  return log;
}

if (process.argv.includes('--write')) {
  writeFileSync(FIXTURE_PATH, `${JSON.stringify(await generate(), null, 1)}\n`);
  console.log(`wrote ${FIXTURE_PATH}`);
}
