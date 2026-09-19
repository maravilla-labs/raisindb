import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./conversation.yaml', import.meta.url), 'utf8');

test('conversations permit messages and AI compaction records', () => {
  assert.match(source, /^\s+- raisin:Message$/m);
  assert.match(source, /^\s+- raisin:AICompaction$/m);
});
