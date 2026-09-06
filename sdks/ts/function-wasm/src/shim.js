// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

/**
 * The QuickJS compatibility shim.
 *
 * `generated/api_wrapper.js` is a byte-identical copy of the QuickJS runtime's
 * own wrapper — the FROZEN public surface, per-method error conventions and
 * all — so this SDK re-implements nothing. It only supplies the three globals
 * that wrapper reaches for, over the WIT host:
 *
 * - `__raisin_call(method, argsJson) -> string`, the registry gateway. It
 *   never throws: a host-side failure becomes the gateway's own error envelope
 *   `{"error":true,"message":"..."}`, exactly as in QuickJS, so each method's
 *   error convention (throw / null / [] / sentinel) still decides.
 * - `__raisin_internal`, the per-execution temp-file helpers backing
 *   `Resource.resize/toImage/getPageCount`. Those need ImageMagick and a
 *   filesystem, neither of which exists here, so every property is a Proxy
 *   that throws by name instead of an `undefined is not a function`.
 * - `console.*`, forwarded to `host.log` (which lands in `ExecutionResult.logs`
 *   and the SSE log stream, same as QuickJS).
 *
 * Then `raisin.context` is filled from `host.context()`.
 */

import { host, SDK_ABI_VERSION } from './host.js';
import './generated/api_wrapper.js';

/** Host functions the wrapper's `Resource` class needs and this runtime lacks. */
const UNAVAILABLE_INTERNAL =
  'not available in WebAssembly functions: the Resource image/PDF helpers ' +
  '(resize, toImage, getPageCount, getBinary on a processed resource) need the ' +
  "server's per-execution temp files. Use raisin.assets.* or a JavaScript " +
  'function for those.';

/** Turn a thrown Component Model error into a message string. */
function errorMessage(e) {
  // jco lifts a `result<_, string>` err arm by throwing a ComponentError whose
  // `payload` is the err value; a plain Error can also reach us.
  if (e && typeof e === 'object' && typeof e.payload === 'string') return e.payload;
  if (e && typeof e === 'object' && typeof e.message === 'string') return e.message;
  return String(e);
}

/** Format console arguments the way a developer expects to read them. */
function formatLogArgs(args) {
  return args
    .map((a) => {
      if (typeof a === 'string') return a;
      if (a instanceof Error) return a.stack || `${a.name}: ${a.message}`;
      try {
        return JSON.stringify(a);
      } catch {
        return String(a);
      }
    })
    .join(' ');
}

function makeConsole() {
  const emit = (level) => (...args) => {
    host().log(level, formatLogArgs(args));
  };
  return {
    log: emit('info'),
    info: emit('info'),
    debug: emit('debug'),
    trace: emit('debug'),
    warn: emit('warn'),
    error: emit('error'),
  };
}

function makeInternalProxy() {
  return new Proxy(
    {},
    {
      get(_target, prop) {
        if (typeof prop === 'symbol') return undefined;
        return () => {
          throw new Error(`__raisin_internal.${String(prop)} is ${UNAVAILABLE_INTERNAL}`);
        };
      },
    },
  );
}

/**
 * Refuse a host whose ABI major version is not the one this SDK was generated
 * for. A host that does not answer `abiVersion` at all (an older mock) passes.
 */
function checkAbi() {
  const h = host();
  if (typeof h.abiVersion !== 'function') return;
  const hostVersion = String(h.abiVersion());
  const major = (v) => String(v).split('.')[0];
  if (major(hostVersion) !== major(SDK_ABI_VERSION)) {
    throw new Error(
      `host ABI ${hostVersion} is incompatible with this SDK (built for ${SDK_ABI_VERSION})`,
    );
  }
}

/**
 * Install the globals a QuickJS function expects, then fill `raisin.context`.
 *
 * Idempotent: the generated component entry calls it once at module scope, and
 * a test may call it again after swapping the mock host.
 */
export function installShim() {
  checkAbi();

  globalThis.__raisin_call = (method, argsJson) => {
    try {
      return host().call(method, argsJson);
    } catch (e) {
      return JSON.stringify({ error: true, message: errorMessage(e) });
    }
  };

  globalThis.__raisin_internal = makeInternalProxy();
  globalThis.console = makeConsole();

  let context = {};
  try {
    context = JSON.parse(host().context() || '{}');
  } catch (e) {
    throw new Error(`host returned an unparseable execution context: ${errorMessage(e)}`);
  }
  globalThis.raisin.context = context;

  return globalThis.raisin;
}
