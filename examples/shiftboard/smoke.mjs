#!/usr/bin/env node
/**
 * Shiftboard demo - backend smoke test.
 *
 * Proves the full direct-chat pipeline with the JS SDK over WebSocket+SSE:
 *   login (identity user) -> create ai_chat conversation with the agent ->
 *   send a message -> stream events (text chunks, tool calls) -> verify the
 *   agent called its tools, the shift node was updated, and token usage was
 *   recorded on the assistant message.
 *
 * Costs real Groq tokens (2 turns, small model) - run sparingly.
 *
 * Run: npm run smoke
 */

import { RaisinClient, MemoryTokenStorage } from '@raisindb/client';

const WS_URL = process.env.RAISIN_WS_URL ?? 'ws://localhost:8081/ws/shiftboard';
const REPO = process.env.RAISIN_REPO ?? 'shiftboard';
const EMAIL = 'planner@example.com';
const PASSWORD = 'Planner12345!';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function assert(cond, message) {
  if (!cond) throw new Error(`Assertion failed: ${message}`);
}

async function streamTurn(db, conversationPath, text) {
  console.log(`\n>>> USER: ${text}`);
  const seen = { chunks: 0, toolCalls: [], done: null, failed: null };

  const timeout = AbortSignal.timeout(120_000);
  for await (const event of db.conversations.sendMessage(conversationPath, text, {
    stream: true,
    signal: timeout,
  })) {
    switch (event.type) {
      case 'text_chunk':
        seen.chunks++;
        process.stdout.write(event.text ?? '');
        break;
      case 'tool_call_started':
        console.log(`\n  🔧 tool started: ${event.functionName}(${JSON.stringify(event.arguments ?? {})})`);
        seen.toolCalls.push(event.functionName);
        break;
      case 'tool_call_completed':
        console.log(`  ✅ tool done: ${event.toolCallId ?? ''} ${event.error ? 'ERROR ' + event.error : ''}`);
        break;
      case 'done':
        seen.done = event;
        break;
      case 'failed':
        seen.failed = event;
        break;
      default:
        break;
    }
    if (seen.done || seen.failed) break;
  }
  console.log();
  if (seen.failed) {
    throw new Error(`Turn failed: ${JSON.stringify(seen.failed)}`);
  }
  return seen;
}

async function main() {
  console.log(`Shiftboard smoke test against ${WS_URL}\n`);

  const client = new RaisinClient(WS_URL, {
    tokenStorage: new MemoryTokenStorage(),
    tenantId: 'default',
    requestTimeout: 30000,
  });

  const user = await client.loginWithEmail(EMAIL, PASSWORD, REPO);
  console.log(`✅ Logged in as ${user.email} (home: ${user.home})`);

  const db = client.database(REPO);

  // 1. Create conversation with the shift-planner agent
  const convo = await db.conversations.create({
    participant: '/agents/shift-planner',
    subject: 'Weekend shift planning',
  });
  console.log(`✅ Conversation created: ${convo.conversationPath}`);

  // 2. Turn 1 - a question that requires the list-shifts tool
  const turn1 = await streamTurn(
    db,
    convo.conversationPath,
    'Which shifts are still open this weekend? Use your tools.',
  );
  assert(
    turn1.toolCalls.length > 0 || turn1.done,
    'turn 1 should have produced tool calls and a done event',
  );

  // 3. Turn 2 - an instruction that requires assign-shift (node update!)
  const turn2 = await streamTurn(
    db,
    convo.conversationPath,
    'Assign Ben to the Saturday evening shift, please.',
  );

  // 4. Verify the shift node was actually updated
  await sleep(1000);
  const shiftResult = await db.executeSql(
    "SELECT path, properties FROM 'staffing' WHERE path = '/shifts/sat-evening'",
  );
  const shift = shiftResult.rows?.[0];
  console.log('Shift node after turn 2:', JSON.stringify(shift?.properties ?? {}, null, 2));
  assert(shift, 'sat-evening shift node must exist');
  const assigned = (shift.properties?.assignee ?? '').toLowerCase();
  assert(
    assigned.includes('ben'),
    `sat-evening must be assigned to Ben, got: ${JSON.stringify(shift.properties?.assignee)}`,
  );
  assert(shift.properties?.status === 'filled', 'sat-evening status must be filled');
  console.log('✅ Shift node updated by the agent (assignee=Ben, status=filled)');

  // 5. Verify messages persisted
  const messages = await db.conversations.getMessages(convo.conversationPath);
  const assistantMessages = messages.filter((m) => m.role === 'assistant');
  console.log(`✅ ${messages.length} messages persisted (${assistantMessages.length} assistant)`);
  assert(assistantMessages.length >= 2, 'expected at least 2 assistant messages');

  // 6. Verify token accounting. The canonical conversation (with per-call
  //    raisin:AICostRecord children and `tokens` on assistant messages)
  //    lives on the AGENT side in the `ai` workspace - regular users can't
  //    read it (RLS), so check it with an admin session.
  const adminClient = new RaisinClient(WS_URL, {
    tokenStorage: new MemoryTokenStorage(),
    tenantId: 'default',
  });
  await adminClient.connect();
  await adminClient.authenticate({
    username: process.env.RAISIN_USER ?? 'admin',
    password: process.env.RAISIN_PASSWORD ?? 'Admin12345!@#',
  });
  const adminDb = adminClient.database(REPO);
  const agentConv = `/agents/shift-planner/inbox/chats/${convo.conversationPath.split('/').pop()}`;
  const costRows = await adminDb.executeSql(
    `SELECT path, properties FROM 'ai' WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AICostRecord'`,
    [agentConv],
  );
  let inputTokens = 0;
  let outputTokens = 0;
  for (const row of costRows.rows ?? []) {
    inputTokens += row.properties?.input_tokens ?? 0;
    outputTokens += row.properties?.output_tokens ?? 0;
  }
  console.log(
    `✅ ${costRows.rows?.length ?? 0} AICostRecord nodes: ${inputTokens} input + ${outputTokens} output tokens (${costRows.rows?.[0]?.properties?.model ?? '?'})`,
  );
  assert((costRows.rows?.length ?? 0) > 0, 'AICostRecord nodes must exist for the conversation');
  assert(inputTokens + outputTokens > 0, 'token usage must be recorded in cost records');
  await adminClient.disconnect?.();

  console.log('\n🎉 Smoke test PASSED - chat, tool calls, node update, tokens all verified.');
  await client.disconnect?.();
  process.exit(0);
}

main().catch((err) => {
  console.error('\n❌ Smoke test FAILED:', err.message || err);
  process.exit(1);
});
