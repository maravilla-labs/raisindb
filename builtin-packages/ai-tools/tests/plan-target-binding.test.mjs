import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const AI = join(dirname(fileURLToPath(import.meta.url)), '../content/functions/lib/raisin/ai');
const { handler: createPlan } = await import(`${AI}/create-plan/index.js`);
const { handler: addTask } = await import(`${AI}/add-task/index.js`);

function host() {
  const store = new Map();
  let sequence = 0;
  globalThis.raisin = {
    nodes: {
      async create(_workspace, parent, input) {
        sequence += 1;
        const path = `${parent}/${input.name}`;
        const node = { id: `n-${sequence}`, path, node_type: input.node_type, properties: input.properties };
        store.set(path, node);
        return node;
      },
      async update(_workspace, path, patch) {
        store.set(path, { ...store.get(path), ...patch });
        return { path };
      },
    },
    sql: {
      async query(sql) {
        if (/raisin:AIPlan/.test(sql)) {
          const plan = [...store.values()].find((node) => node.node_type === 'raisin:AIPlan');
          return plan ? [plan] : [];
        }
        if (/COUNT\(\*\)/.test(sql)) {
          return [{ count: [...store.values()].filter((node) => node.node_type === 'raisin:AITask').length }];
        }
        return [];
      },
    },
  };
  return store;
}

test('create-plan ignores speculative build targets until an artifact exists', async () => {
  const store = host();
  await createPlan({
    title: 'Build automation',
    tasks: [{ title: 'Discover or reuse agent', build_target_path: '/agents/emoji-decider' }],
    __raisin_context: { workspace: 'ai', msg_path: '/chat/reply', execution_mode: 'approve_then_auto' },
  });

  const task = [...store.values()].find((node) => node.node_type === 'raisin:AITask');
  assert.equal(task.properties.build_target_path, undefined);
});

test('add-task also ignores plan-time target guesses', async () => {
  const store = host();
  await createPlan({
    title: 'Build automation',
    tasks: [{ title: 'Design' }],
    __raisin_context: { workspace: 'ai', msg_path: '/chat/reply', execution_mode: 'automatic' },
  });
  await addTask({
    title: 'Maybe create an agent',
    build_target_path: '/agents/guessed-agent',
    __raisin_context: { workspace: 'ai', chat_path: '/chat' },
  });

  const tasks = [...store.values()].filter((node) => node.node_type === 'raisin:AITask');
  assert.equal(tasks.at(-1).properties.build_target_path, undefined);
});
