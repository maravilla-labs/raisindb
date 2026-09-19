import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./ai.yaml', import.meta.url), 'utf8');
const compactionSource = await readFile(new URL('../nodetypes/ai_compaction.yaml', import.meta.url), 'utf8');
const costRecordSource = await readFile(new URL('../nodetypes/ai_cost_record.yaml', import.meta.url), 'utf8');

test('AI workspace permits modern messaging and compaction nodes', () => {
  for (const nodeType of [
    'raisin:Conversation',
    'raisin:Message',
    'raisin:AICompaction',
  ]) {
    assert.match(source, new RegExp(`^\\s+- ${nodeType}$`, 'm'));
  }
});

test('compaction permits its accounting record as a child', () => {
  assert.match(compactionSource, /^\s+- raisin:AICostRecord$/m);
  assert.match(costRecordSource, /^\s+- raisin:AICompaction$/m);
});
