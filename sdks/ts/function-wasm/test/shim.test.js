// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

// The point of these tests: the `raisin.*` surface a wasm function sees is the
// QuickJS surface, byte for byte, because it IS `quickjs/api_wrapper.js`. They
// run natively — no jco, no wasm, no server.

import { beforeEach, describe, expect, it } from 'vitest';
import { createMockHost } from '../src/testing.js';
import { clearHost, setHost } from '../src/host.js';
import { installShim } from '../src/shim.js';

describe('the QuickJS shim over the WIT host', () => {
  beforeEach(() => clearHost());

  it('dispatches raisin.* through host.call with positional JSON args', () => {
    const mock = createMockHost();
    mock.expect('nodes_get', (args) => {
      expect(args).toEqual(['content', '/people/ada']);
      return { path: '/people/ada', properties: { title: 'Ada' } };
    });
    mock.install();

    const node = globalThis.raisin.nodes.get('content', '/people/ada');

    expect(node.properties.title).toBe('Ada');
    expect(mock.calls[0].method).toBe('nodes_get');
  });

  it('keeps each method its own error convention when the host fails', () => {
    const mock = createMockHost();
    mock.expect('nodes_get', () => {
      throw new Error('node not found');
    });
    mock.expect('nodes_create', () => {
      throw new Error('permission denied');
    });
    mock.expect('nodes_getChildren', () => {
      throw new Error('nope');
    });
    mock.install();

    // The frozen conventions: get -> null, getChildren -> [], create -> throw.
    expect(globalThis.raisin.nodes.get('content', '/missing')).toBeNull();
    expect(globalThis.raisin.nodes.getChildren('content', '/missing')).toEqual([]);
    expect(() => globalThis.raisin.nodes.create('content', '/x', {})).toThrow(
      /permission denied/,
    );
  });

  it('fills raisin.context from host.context()', () => {
    const mock = createMockHost({
      context: { tenant_id: 't1', repo_id: 'r1', branch: 'main', input: { name: 'Ada' } },
    });
    mock.install();

    expect(globalThis.raisin.context.tenant_id).toBe('t1');
    expect(globalThis.raisin.context.input.name).toBe('Ada');
  });

  it('forwards console.* to host.log with a level each', () => {
    const mock = createMockHost();
    mock.install();

    console.log('hello', { n: 1 });
    console.warn('careful');
    console.error(new Error('boom'));
    console.debug('noisy');

    expect(mock.logs.map((l) => l.level)).toEqual(['info', 'warn', 'error', 'debug']);
    expect(mock.logs[0].message).toBe('hello {"n":1}');
    expect(mock.logs[2].message).toMatch(/boom/);
  });

  it('names the missing Resource helper instead of "undefined is not a function"', () => {
    createMockHost().install();

    expect(() => globalThis.__raisin_internal.temp_resize('h', '{}')).toThrow(
      /temp_resize is not available in WebAssembly functions/,
    );
  });

  it('turns an unknown host method into the gateway error envelope', () => {
    createMockHost().install();

    const raw = globalThis.__raisin_call('nodes_get', JSON.stringify(['content', '/a']));

    expect(JSON.parse(raw)).toEqual({
      error: true,
      message: "unexpected host call 'nodes_get'",
    });
  });

  it('refuses a host whose ABI major version differs', () => {
    setHost(createMockHost({ abiVersion: '1.0.0' }).host);

    expect(() => installShim()).toThrow(/host ABI 1\.0\.0 is incompatible/);
  });

  it('explains itself when no host is installed', () => {
    expect(() => installShim()).toThrow(/no RaisinDB host is installed/);
  });
});
