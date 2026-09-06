// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

/**
 * A mock host, so the whole SDK surface can be exercised natively.
 *
 * `vitest` runs against this: no jco, no wasm, no server. It is the same
 * bargain the Rust and Go SDKs make (`MockHost::expect`) — the WIT host is one
 * small interface, so faking it costs a page and buys a test suite that runs in
 * milliseconds.
 *
 * The registry gateway's contract is reproduced faithfully, because that is
 * what the frozen per-method error conventions in `api_wrapper.js` react to:
 * a `call` answers with the JSON encoding of `InvokeResult` (a `void` method
 * answers literal `true`), and a failure THROWS the way the Component Model
 * `result<string, string>` err arm does.
 */

import { setHost } from './host.js';
import { installShim } from './shim.js';

/**
 * @typedef {object} MockHostOptions
 * @property {Record<string, unknown>} [context] execution context JSON value
 * @property {string} [abiVersion] host ABI, default "0.1.0"
 */

/**
 * Create a mock host.
 *
 * @param {MockHostOptions} [options]
 */
export function createMockHost(options = {}) {
  const context = options.context ?? {};
  const abiVersion = options.abiVersion ?? '0.1.0';

  /** @type {Array<{method: string, args: unknown[]}>} */
  const calls = [];
  /** @type {Array<{level: string, message: string}>} */
  const logs = [];
  /** @type {Map<string, (args: unknown[]) => unknown>} */
  const expectations = new Map();

  const host = {
    call(method, argsJson) {
      const args = JSON.parse(argsJson);
      calls.push({ method, args });
      const impl = expectations.get(method);
      if (!impl) {
        // An unexpected call is the err arm, not a silent undefined: that is
        // what makes a typo'd method name fail the test that made it.
        throw new Error(`unexpected host call '${method}'`);
      }
      return JSON.stringify(impl(args));
    },
    log(level, message) {
      logs.push({ level, message });
    },
    context() {
      return JSON.stringify(context);
    },
    abiVersion() {
      return abiVersion;
    },
  };

  return {
    /** The `RaisinHost` implementation to hand to `setHost`. */
    host,
    /** Every `call` made, in order. */
    calls,
    /** Every `console.*` line, in order. */
    logs,
    /**
     * Answer `method` with `result` (or with `result(args)` if it is a
     * function). A method that must fail should throw from that function.
     */
    expect(method, result) {
      expectations.set(method, typeof result === 'function' ? result : () => result);
      return this;
    },
    /** Make this the installed host and run the QuickJS shim against it. */
    install() {
      setHost(host);
      return installShim();
    },
  };
}

/**
 * Install a fresh mock host, run `fn` against it, and return its result.
 *
 * @template T
 * @param {(mock: ReturnType<typeof createMockHost>) => T} fn
 * @param {MockHostOptions} [options]
 */
export function withMockHost(fn, options) {
  const mock = createMockHost(options);
  mock.install();
  return fn(mock);
}
