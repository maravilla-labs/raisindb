// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

/**
 * The name-routed component export.
 *
 * The WIT world exports exactly one function,
 * `handler: func(name: string, input: string) -> result<string, string>`.
 * `name` is the handler selected by the Function node's `entry_file` suffix
 * (`main.wasm:on-order` -> `"on-order"`; a bare `main.wasm` -> `"default"`), so
 * ONE artifact — 8-12 MB of StarlingMonkey — can back many Function nodes.
 *
 * Routing mirrors the `index.js:handlerName` grammar QuickJS functions already
 * use: `"default"` resolves the module's `handler` export, any other name the
 * export of that name. The host never validates the name; an unknown one is
 * answered here with an Err listing what this module actually exports.
 */

import { installShim } from './shim.js';

/** JS identifier for a kebab-case handler name: `on-order` -> `onOrder`. */
function camelCase(name) {
  return name.replace(/-([a-z0-9])/g, (_, c) => c.toUpperCase());
}

/** The handler names this module offers, in the spelling `entry_file` uses. */
export function registeredHandlers(module) {
  const names = [];
  for (const key of Object.keys(module)) {
    if (typeof module[key] !== 'function') continue;
    names.push(key === 'handler' ? 'default' : key);
  }
  if (!names.includes('default') && typeof module.default === 'function') {
    names.unshift('default');
  }
  return names;
}

/** Resolve one handler name against a module's exports, or `undefined`. */
export function resolveHandler(module, name) {
  if (name === 'default') {
    if (typeof module.handler === 'function') return module.handler;
    if (typeof module.default === 'function') return module.default;
    return undefined;
  }
  if (typeof module[name] === 'function') return module[name];
  // `on-order` is not a legal JS identifier, so also accept the camel-case
  // spelling authors actually write (`export function onOrder(input)`).
  const camel = camelCase(name);
  if (camel !== name && typeof module[camel] === 'function') return module[camel];
  return undefined;
}

function messageOf(e) {
  if (e && typeof e === 'object' && typeof e.message === 'string') return e.message;
  return String(e);
}

/**
 * Build the `handler(name, input)` export from a user module.
 *
 * The shim is installed on invocation, never at module scope: ComponentizeJS
 * pre-initializes the component by EVALUATING the top level with Wizer at build
 * time, where no host exists and `host.context()` would be a build failure.
 *
 * @param {Record<string, unknown>} module the user's module namespace object
 * @returns {(name: string, input: string) => Promise<string>}
 */
export function createHandler(module) {
  return async function handler(name, input) {
    const fn = resolveHandler(module, name);
    if (!fn) {
      const registered = registeredHandlers(module);
      throw new Error(
        `unknown handler '${name}'; this component registered: ` +
          (registered.length ? registered.join(', ') : '(none)'),
      );
    }

    installShim();

    let parsed;
    try {
      parsed = input === undefined || input === null || input === '' ? {} : JSON.parse(input);
    } catch (e) {
      throw new Error(`invalid input JSON: ${messageOf(e)}`);
    }

    let result;
    try {
      result = await fn(parsed);
    } catch (e) {
      // Normalize: jco lowers the err arm as a string, and a thrown non-Error
      // would otherwise reach it as something it cannot lower.
      throw new Error(messageOf(e));
    }

    const encoded = JSON.stringify(result === undefined ? null : result);
    return encoded === undefined ? 'null' : encoded;
  };
}
