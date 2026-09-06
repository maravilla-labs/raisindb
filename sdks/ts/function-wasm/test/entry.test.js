// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

// One artifact, many handlers: these tests pin the name routing, which is the
// whole reason the WIT export takes a `name` at all.

import { beforeEach, describe, expect, it } from 'vitest';
import { createHandler, registeredHandlers, resolveHandler } from '../src/entry.js';
import { createMockHost } from '../src/testing.js';
import { clearHost } from '../src/host.js';

const userModule = {
  async handler(input) {
    return { greeting: `Hello, ${input.name}!` };
  },
  async onOrder(input) {
    console.log('order', input.id);
    return { ok: true, id: input.id };
  },
  notAFunction: 42,
};

describe('name-routed handler(name, input)', () => {
  beforeEach(() => {
    clearHost();
    createMockHost().install();
  });

  it("routes 'default' to the module's handler export", async () => {
    const handler = createHandler(userModule);

    const out = await handler('default', JSON.stringify({ name: 'Ada' }));

    expect(JSON.parse(out)).toEqual({ greeting: 'Hello, Ada!' });
  });

  it('routes a kebab-case entry_file suffix to its camelCase export', async () => {
    const handler = createHandler(userModule);

    const out = await handler('on-order', JSON.stringify({ id: 7 }));

    expect(JSON.parse(out)).toEqual({ ok: true, id: 7 });
  });

  it('routes an exact export name too', async () => {
    const handler = createHandler(userModule);

    expect(JSON.parse(await handler('onOrder', '{"id":1}')).id).toBe(1);
  });

  it('answers an unknown name by listing what the component registered', async () => {
    const handler = createHandler(userModule);

    await expect(handler('on-shipment', '{}')).rejects.toThrow(
      /unknown handler 'on-shipment'; this component registered: default, onOrder/,
    );
  });

  it("falls back to a default export for 'default'", async () => {
    const handler = createHandler({ default: async () => ({ ok: true }) });

    expect(JSON.parse(await handler('default', '{}'))).toEqual({ ok: true });
  });

  it('reports a bad input document as an error, not a crash', async () => {
    const handler = createHandler(userModule);

    await expect(handler('default', '{not json')).rejects.toThrow(/invalid input JSON/);
  });

  it('normalizes anything a handler throws into a message', async () => {
    const handler = createHandler({
      handler() {
        throw { code: 'E_NOPE', message: 'nope' };
      },
    });

    await expect(handler('default', '{}')).rejects.toThrow('nope');
  });

  it('encodes an undefined return as null, not as a missing value', async () => {
    const handler = createHandler({ handler() {} });

    expect(await handler('default', '{}')).toBe('null');
  });

  it('installs the shim lazily, so Wizer never calls the host at build time', async () => {
    clearHost();
    const mock = createMockHost();
    // Creating the handler must not touch the host: componentize-js evaluates
    // the module top level with Wizer, where there is none.
    const handler = createHandler(userModule);
    expect(mock.calls).toHaveLength(0);

    mock.install();
    await handler('default', '{"name":"Ada"}');
  });
});

describe('handler resolution helpers', () => {
  it('lists handler names in entry_file spelling', () => {
    expect(registeredHandlers(userModule)).toEqual(['default', 'onOrder']);
  });

  it('resolves nothing for a name the module does not export', () => {
    expect(resolveHandler(userModule, 'missing')).toBeUndefined();
  });
});
