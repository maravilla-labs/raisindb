// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

// Native tests: no wasm, no jco, no server. The mock host stands in for the
// WIT gateway, so the same source that ships as a component is exercised here
// in milliseconds.

import { beforeEach, describe, expect, it } from 'vitest';
import { createHandler } from '@raisindb/function-wasm';
import { createMockHost } from '@raisindb/function-wasm/testing';
import * as fn from '../src/index.js';

const handler = createHandler(fn);

let mock;
beforeEach(() => {
  mock = createMockHost({ context: { tenant_id: 'default', branch: 'main' } });
  mock.expect('nodes_getChildren', () => [
    { path: '/people/ada' },
    { path: '/people/grace' },
  ]);
  mock.install();
});

describe('greet-ts', () => {
  it('greets and counts the people', async () => {
    const out = JSON.parse(await handler('default', JSON.stringify({ name: 'Ada' })));

    expect(out).toEqual({ greeting: 'Hello, Ada!', people: 2, language: 'ts' });
    expect(mock.logs[0]).toEqual({ level: 'info', message: 'greeting Ada (2 people)' });
  });

  it('serves the shout handler from the same artifact', async () => {
    const out = JSON.parse(await handler('shout', JSON.stringify({ name: 'Ada' })));

    expect(out.greeting).toBe('HELLO, ADA!');
  });

  it('rejects a missing name', async () => {
    await expect(handler('default', '{}')).rejects.toThrow('input.name is required');
  });

  it('lists its handlers when the entry_file names one it does not have', async () => {
    await expect(handler('whisper', '{}')).rejects.toThrow(
      /registered: default, shout/,
    );
  });
});
