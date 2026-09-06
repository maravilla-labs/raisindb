// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

/**
 * The WIT host, held in one place.
 *
 * The real host arrives as a Component Model import (`raisin:function/host`),
 * which only resolves inside `jco componentize`. Node — and therefore vitest —
 * cannot resolve that specifier at all, so the import lives in its own module
 * (`host-wit.js`, pulled in only by the generated component entry) and
 * everything else in this SDK goes through the holder below. That is what lets
 * the whole surface be tested natively against a mock host.
 *
 * @typedef {object} RaisinHost
 * @property {(method: string, args: string) => string} call
 *   Registry gateway. `args` is a JSON array of positional arguments; the
 *   return is `InvokeResult::to_json_string()`. A host-side failure THROWS
 *   (the Component Model `result<string, string>` err arm).
 * @property {(level: 'debug'|'info'|'warn'|'error', message: string) => void} log
 * @property {() => string} context   Execution context, as JSON.
 * @property {() => string} abiVersion Host ABI semver, e.g. "0.1.0".
 */

/** The ABI version this SDK was generated against. */
export const SDK_ABI_VERSION = '0.1.0';

/** @type {RaisinHost | null} */
let current = null;

/**
 * Install the host implementation. Called once by `host-wit.js` in a real
 * component, and by tests with a mock.
 *
 * @param {RaisinHost} host
 */
export function setHost(host) {
  if (!host || typeof host.call !== 'function') {
    throw new TypeError('setHost() needs a host with a call(method, args) function');
  }
  current = host;
}

/** Forget the installed host. Test-only; a component never calls it. */
export function clearHost() {
  current = null;
}

/**
 * The installed host.
 *
 * @returns {RaisinHost}
 */
export function host() {
  if (!current) {
    throw new Error(
      'no RaisinDB host is installed: a component entry must import ' +
        "'@raisindb/function-wasm/host-wit', a test must call setHost(mockHost)",
    );
  }
  return current;
}

/** True when a host is installed. */
export function hasHost() {
  return current !== null;
}
