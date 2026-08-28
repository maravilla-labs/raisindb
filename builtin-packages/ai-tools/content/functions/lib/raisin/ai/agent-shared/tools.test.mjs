import assert from 'node:assert/strict';
import test from 'node:test';

import { normalizeCompletionResponse } from './tools.js';

test('normalizeCompletionResponse converts accumulated reasoning to blocks', () => {
  const response = normalizeCompletionResponse({
    content: 'Visible answer',
    thinking: ' private reasoning ',
  });

  assert.equal(response.content, 'Visible answer');
  assert.deepEqual(response.thinking, ['private reasoning']);
});

test('normalizeCompletionResponse preserves multiple non-empty reasoning blocks', () => {
  const response = normalizeCompletionResponse({
    content: '',
    thinking: ['first', '', ' second '],
  });

  assert.deepEqual(response.thinking, ['first', 'second']);
});
