// Canonical-ABI plumbing for the RaisinDB guest world.
//
// AssemblyScript does not implement the Component Model, so this file is what
// `wit-bindgen` would emit for a supported language: the lowering between
// AssemblyScript values and the ABI `wasm-tools component new` expects. It is
// the only file in this SDK that knows about pointers, and the only one anyone
// should need to review when the WIT world changes.
//
// Two rules it exists to get right, both of which fail QUIETLY when wrong:
//
//  1. An imported interface is named WITH ITS PACKAGE VERSION —
//     `raisin:function/host@0.1.0`. Without it, componentization refuses to
//     resolve the import.
//  2. A variant discriminant is stored in the SMALLEST integer that fits its
//     case count, then padded to the payload's alignment. `result<string,
//     string>` is therefore `{ u8 tag, 3 bytes padding, i32 ptr, i32 len }`.
//     Reading the tag as an i32 picks up padding and turns every Ok into an
//     Err while the payload still decodes perfectly — a failure that looks
//     like the host misbehaving rather than like a guest bug.

// --- host imports -----------------------------------------------------------

@external("raisin:function/host@0.1.0", "call")
declare function hostCall(mp: i32, ml: i32, ap: i32, al: i32, ret: i32): void;

@external("raisin:function/host@0.1.0", "log")
declare function hostLog(level: i32, mp: i32, ml: i32): void;

@external("raisin:function/host@0.1.0", "context")
declare function hostContext(ret: i32): void;

@external("raisin:function/host@0.1.0", "abi-version")
declare function hostAbiVersion(ret: i32): void;

// --- memory -----------------------------------------------------------------

/**
 * The host allocates into guest memory through this.
 * `wasm-tools component new` looks it up BY NAME, so it must be re-exported
 * from the guest's entry module.
 */
export function cabi_realloc(oldPtr: i32, oldSize: i32, align: i32, newSize: i32): i32 {
  if (newSize == 0) return align;
  const p = heap.alloc(<usize>newSize);
  if (oldPtr != 0 && oldSize > 0) {
    memory.copy(p, <usize>oldPtr, <usize>(oldSize < newSize ? oldSize : newSize));
  }
  return <i32>p;
}

/** A UTF-8 copy of `s` in linear memory that outlives this call. */
function encode(s: string): usize {
  const buf = String.UTF8.encode(s);
  const p = heap.alloc(<usize>buf.byteLength);
  memory.copy(p, changetype<usize>(buf), <usize>buf.byteLength);
  return p;
}

function byteLen(s: string): i32 {
  return <i32>String.UTF8.byteLength(s);
}

function decode(ptr: i32, len: i32): string {
  if (len <= 0) return "";
  return String.UTF8.decodeUnsafe(<usize>ptr, <usize>len, false);
}

// --- host calls -------------------------------------------------------------

/** Raised when the host answers a `call` with the error arm. */
export class HostError {
  constructor(public message: string) {}
}

/**
 * Invoke a RaisinDB API method by its registry name, e.g. `nodes_getChildren`.
 * `argsJson` is a JSON array of positional arguments.
 *
 * Returns the raw JSON the host produced. Throws `HostError` on the error arm.
 */
export function call(method: string, argsJson: string): string {
  const area = heap.alloc(12);
  hostCall(<i32>encode(method), byteLen(method), <i32>encode(argsJson), byteLen(argsJson), <i32>area);
  const tag = load<u8>(area); // u8 — see the header
  const body = decode(load<i32>(area + 4), load<i32>(area + 8));
  if (tag != 0) throw new HostError(body);
  return body;
}

export namespace log {
  export function debug(message: string): void { emit(0, message); }
  export function info(message: string): void { emit(1, message); }
  export function warn(message: string): void { emit(2, message); }
  export function error(message: string): void { emit(3, message); }
  function emit(level: i32, message: string): void {
    hostLog(level, <i32>encode(message), byteLen(message));
  }
}

/** The execution context as JSON — tenant, repo, branch, actor, execution id. */
export function context(): string {
  const area = heap.alloc(8);
  hostContext(<i32>area);
  return decode(load<i32>(area), load<i32>(area + 4));
}

/** The host ABI version this server speaks, e.g. "0.1.0". */
export function abiVersion(): string {
  const area = heap.alloc(8);
  hostAbiVersion(<i32>area);
  return decode(load<i32>(area), load<i32>(area + 4));
}

// --- the exported handler ---------------------------------------------------

/** A handler: takes the JSON input, returns the JSON output. */
export type Route = (name: string, input: string) => string;

let RET: usize = 0;

function finish(tag: u8, body: string): i32 {
  if (!RET) RET = heap.alloc(12);
  store<u8>(RET, tag);
  store<u8>(RET + 1, 0);
  store<u8>(RET + 2, 0);
  store<u8>(RET + 3, 0);
  store<i32>(RET + 4, <i32>encode(body));
  store<i32>(RET + 8, byteLen(body));
  return <i32>RET;
}

/**
 * Lower the exported `handler(name, input) -> result<string, string>`.
 *
 * A guest's entry module re-exports this as `handler` and hands it a router.
 * A `HostError` (or any thrown error) becomes the ABI's error arm rather than
 * a trap, so a failing API call reports a message instead of killing the
 * instance.
 */
export function run(
  namePtr: i32, nameLen: i32, inputPtr: i32, inputLen: i32, route: Route
): i32 {
  const name = decode(namePtr, nameLen);
  const input = decode(inputPtr, inputLen);
  let out: string;
  if (name.length == 0) return finish(1, "handler name was empty");
  out = route(name, input);
  return finish(0, out);
}

/** The error a router should return for a name it does not know. */
export function unknownHandler(name: string, registered: string): string {
  return '{"error":"unknown handler ' + name + '; registered: ' + registered + '"}';
}
