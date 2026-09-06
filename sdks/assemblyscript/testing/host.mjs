// A mock host for AssemblyScript guests, so a function can be unit-tested with
// no server and no network — the same guarantee the Rust and Go SDKs give.
//
// It loads the CORE module (before `wasm-tools` wraps it as a component) and
// supplies `raisin:function/host` from JavaScript. That means this file speaks
// the Component Model's canonical ABI from the other side: it writes string
// arguments into guest memory through the guest's own `cabi_realloc`, and
// reads results back out of the return area.
//
// Layout it depends on, matching `assembly/abi.ts`:
//   result<string, string>  ->  { u8 tag, 3 bytes padding, i32 ptr, i32 len }
//   string                  ->  { i32 ptr, i32 len }
// The tag is a u8 because the canonical ABI stores a variant discriminant in
// the smallest integer that fits its case count.

import { readFile } from 'node:fs/promises';

const HOST = 'raisin:function/host@0.1.0';
const LEVELS = ['debug', 'info', 'warn', 'error'];

/**
 * Instantiate a guest with a scripted host.
 *
 * @param {string} corePath  the `.wasm` produced by `asc` (not the component)
 * @param {object} options
 * @param {(method: string, argsJson: string) => unknown} [options.call]
 *        Answer a `raisin.*` call. Return a value to send the Ok arm; throw to
 *        send Err. The default throws, so a call the test did not plan for
 *        fails loudly instead of returning something plausible.
 * @param {object} [options.context]  what `context()` returns.
 */
export async function loadGuest(corePath, options = {}) {
  const bytes = await readFile(corePath);
  const logs = [];
  const calls = [];
  let exports;

  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  const mem = () => new Uint8Array(exports.memory.buffer);
  const view = () => new DataView(exports.memory.buffer);

  const readString = (ptr, len) => decoder.decode(mem().subarray(ptr, ptr + len));

  /** Copy a string into guest memory using its own allocator. */
  const writeString = (s) => {
    const buf = encoder.encode(s);
    const ptr = exports.cabi_realloc(0, 0, 1, buf.length);
    mem().set(buf, ptr);
    return [ptr, buf.length];
  };

  /** Write `{ptr, len}` into a return area the guest gave us. */
  const writeStringAt = (area, s) => {
    const [ptr, len] = writeString(s);
    view().setInt32(area, ptr, true);
    view().setInt32(area + 4, len, true);
  };

  const call = options.call ?? ((method) => {
    throw new Error(
      `unexpected host call ${method}. Script it with { call: (method, args) => ... }`
    );
  });

  const imports = {
    [HOST]: {
      call(mp, ml, ap, al, ret) {
        const method = readString(mp, ml);
        const args = readString(ap, al);
        calls.push({ method, args: JSON.parse(args) });
        const v = view();
        try {
          const result = call(method, args);
          const body = typeof result === 'string' ? result : JSON.stringify(result ?? null);
          v.setUint8(ret, 0); // ok
          const [ptr, len] = writeString(body);
          v.setInt32(ret + 4, ptr, true);
          v.setInt32(ret + 8, len, true);
        } catch (error) {
          v.setUint8(ret, 1); // err
          const [ptr, len] = writeString(String(error?.message ?? error));
          v.setInt32(ret + 4, ptr, true);
          v.setInt32(ret + 8, len, true);
        }
      },
      log(level, mp, ml) {
        logs.push({ level: LEVELS[level] ?? String(level), message: readString(mp, ml) });
      },
      context(ret) {
        writeStringAt(ret, JSON.stringify(options.context ?? { tenant_id: 'test' }));
      },
      'abi-version': (ret) => writeStringAt(ret, '0.1.0'),
    },
  };

  const { instance } = await WebAssembly.instantiate(bytes, imports);
  exports = instance.exports;

  return {
    /** Call a handler by name. Returns the parsed JSON it produced. */
    invoke(name, input) {
      const [np, nl] = writeString(name);
      const [ip, il] = writeString(typeof input === 'string' ? input : JSON.stringify(input));
      const ret = exports.handler(np, nl, ip, il);
      const v = view();
      const tag = v.getUint8(ret); // u8 — see the header
      const body = readString(v.getInt32(ret + 4, true), v.getInt32(ret + 8, true));
      if (tag !== 0) throw new Error(body);
      return JSON.parse(body);
    },
    /** Every log line the guest emitted, as `{ level, message }`. */
    logs,
    /** Every host call the guest made, as `{ method, args }`. */
    calls,
  };
}
