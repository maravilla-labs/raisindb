// AssemblyScript guest, hand-lowered to the Component Model canonical ABI.
//
// `handler: func(name: string, input: string) -> result<string, string>`
// flattens to (i32 namePtr, i32 nameLen, i32 inputPtr, i32 inputLen) and,
// because the result needs more than one flat value, returns a POINTER to a
// return area laid out as { i32 tag, i32 ptr, i32 len } (tag 0 = ok, 1 = err).

/** Return area, allocated once. */
let RET: usize = 0;

/**
 * The host allocates into guest memory through this. Required by the ABI:
 * `component new` looks for it by name.
 */
export function cabi_realloc(oldPtr: i32, oldSize: i32, align: i32, newSize: i32): i32 {
  if (newSize == 0) return align;
  const p = heap.alloc(<usize>newSize);
  if (oldPtr != 0 && oldSize > 0) {
    memory.copy(p, <usize>oldPtr, <usize>(oldSize < newSize ? oldSize : newSize));
  }
  return <i32>p;
}

/** Write a UTF-8 byte sequence we own, and return (ptr,len) through the area. */
function ok(bytes: usize, len: i32): i32 {
  if (!RET) RET = heap.alloc(12);
  store<i32>(RET, 0);
  store<i32>(RET + 4, <i32>bytes);
  store<i32>(RET + 8, len);
  return <i32>RET;
}

export function handler(namePtr: i32, nameLen: i32, inputPtr: i32, inputLen: i32): i32 {
  // Prove both string params arrive intact: echo `{"handler":"<name>","echo":<input>}`.
  const prefix = '{"handler":"';
  const middle = '","echo":';
  const suffix = '}';

  const pBytes = String.UTF8.encode(prefix);
  const mBytes = String.UTF8.encode(middle);
  const sBytes = String.UTF8.encode(suffix);
  const total = pBytes.byteLength + nameLen + mBytes.byteLength + inputLen + sBytes.byteLength;

  const out = heap.alloc(<usize>total);
  let at = out;
  memory.copy(at, changetype<usize>(pBytes), <usize>pBytes.byteLength); at += <usize>pBytes.byteLength;
  memory.copy(at, <usize>namePtr, <usize>nameLen);                     at += <usize>nameLen;
  memory.copy(at, changetype<usize>(mBytes), <usize>mBytes.byteLength); at += <usize>mBytes.byteLength;
  memory.copy(at, <usize>inputPtr, <usize>inputLen);                   at += <usize>inputLen;
  memory.copy(at, changetype<usize>(sBytes), <usize>sBytes.byteLength);

  return ok(out, total);
}
