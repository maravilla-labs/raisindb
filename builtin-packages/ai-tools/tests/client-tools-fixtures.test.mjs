/** The committed Rust-decoder fixture is what the JS reducer produces today. */
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { generate, FIXTURE_PATH } from './client-tools-fixtures.mjs';

test('crates/raisin-agent-contract/fixtures/js-reducer/client-tools.json is current', async () => {
  const committed = JSON.parse(readFileSync(FIXTURE_PATH, 'utf8'));
  assert.deepEqual(committed, await generate(), 'regenerate: node builtin-packages/ai-tools/tests/client-tools-fixtures.mjs --write');
});
