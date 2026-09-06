// AssemblyScript guest — canonical ABI, EXPORT and IMPORT sides.
//
// Imports of `raisin:function/host`. A result that needs more than one flat
// value is returned indirectly: the CALLER (this guest) provides a return-area
// pointer and the host writes into it.
//
//   call:        (methodPtr, methodLen, argsPtr, argsLen, retArea) -> void
//                retArea = { u8 tag, pad, pad, pad, i32 ptr, i32 len }
//
// The discriminant is a U8, not an i32: the canonical ABI stores a variant tag
// in the smallest integer that fits its case count, then pads to the payload's
// alignment. Reading it as i32 picks up three bytes of padding and every Ok
// looks like an Err — with the payload still decoding correctly, which is what
// makes the mistake quiet.
//   log:         (level, msgPtr, msgLen) -> void           level is the enum's i32
//   context:     (retArea) -> void                          retArea = { i32 ptr, i32 len }
//   abi-version: (retArea) -> void

@external("raisin:function/host@0.1.0", "call")
declare function hostCall(mp: i32, ml: i32, ap: i32, al: i32, ret: i32): void;

@external("raisin:function/host@0.1.0", "log")
declare function hostLog(level: i32, mp: i32, ml: i32): void;

@external("raisin:function/host@0.1.0", "context")
declare function hostContext(ret: i32): void;

let RET: usize = 0;

export function cabi_realloc(oldPtr: i32, oldSize: i32, align: i32, newSize: i32): i32 {
  if (newSize == 0) return align;
  const p = heap.alloc(<usize>newSize);
  if (oldPtr != 0 && oldSize > 0) {
    memory.copy(p, <usize>oldPtr, <usize>(oldSize < newSize ? oldSize : newSize));
  }
  return <i32>p;
}

/** Copy an AssemblyScript string into linear memory as UTF-8, returning (ptr,len). */
function utf8(s: string): usize {
  const buf = String.UTF8.encode(s);
  const p = heap.alloc(<usize>buf.byteLength);
  memory.copy(p, changetype<usize>(buf), <usize>buf.byteLength);
  return p;
}
function utf8len(s: string): i32 {
  return <i32>String.UTF8.byteLength(s);
}

/** Read a (ptr,len) pair the host wrote as an AssemblyScript string. */
function readStr(ptr: i32, len: i32): string {
  return String.UTF8.decodeUnsafe(<usize>ptr, <usize>len, false);
}

export function handler(namePtr: i32, nameLen: i32, inputPtr: i32, inputLen: i32): i32 {
  const area = heap.alloc(16);

  // 1. log — no return value, the simplest import.
  const msg = "assemblyscript guest running";
  hostLog(1 /* info */, <i32>utf8(msg), utf8len(msg));

  // 2. context() -> string, returned indirectly as (ptr, len).
  hostContext(<i32>area);
  const ctx = readStr(load<i32>(area), load<i32>(area + 4));

  // 3. call() -> result<string,string>, returned as (tag, ptr, len).
  const method = "nodes_getChildren";
  const args = '["content","/pages",3]';
  hostCall(<i32>utf8(method), utf8len(method), <i32>utf8(args), utf8len(args), <i32>area);
  const tag = load<u8>(area);
  const body = readStr(load<i32>(area + 4), load<i32>(area + 8));

  const name = readStr(namePtr, nameLen);
  const input = readStr(inputPtr, inputLen);
  const out =
    '{"handler":"' + name + '","echo":' + input +
    ',"ctx_has_tenant":' + (ctx.includes("tenant") ? "true" : "false") +
    ',"call_ok":' + (tag == 0 ? "true" : "false") +
    ',"children":' + body + '}';

  if (!RET) RET = heap.alloc(12);
  store<u8>(RET, 0); // ok — a u8 tag, for the same reason as above
  store<u8>(RET + 1, 0);
  store<u8>(RET + 2, 0);
  store<u8>(RET + 3, 0);
  store<i32>(RET + 4, <i32>utf8(out));
  store<i32>(RET + 8, utf8len(out));
  return <i32>RET;
}
