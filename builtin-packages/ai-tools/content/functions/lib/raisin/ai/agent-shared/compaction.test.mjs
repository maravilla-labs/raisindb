import assert from 'node:assert/strict';
import test from 'node:test';

import { tokensSinceLatestCompaction } from './compaction.js';

test('token window uses lifetime total before the first compaction', () => {
  assert.equal(tokensSinceLatestCompaction(91_067, null), 91_067);
});

test('token window starts from the latest compaction checkpoint', () => {
  const compaction = { properties: { token_checkpoint: 52_500 } };
  assert.equal(tokensSinceLatestCompaction(61_000, compaction), 8_500);
});

test('token window never becomes negative for stale totals', () => {
  const compaction = { properties: { token_checkpoint: 52_500 } };
  assert.equal(tokensSinceLatestCompaction(50_000, compaction), 0);
});
