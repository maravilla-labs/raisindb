# @raisindb/function-assemblyscript

Write a RaisinDB server function in AssemblyScript and ship it as a WebAssembly
component — TypeScript-shaped syntax, no embedded JavaScript engine, artifacts
measured in kilobytes.

```bash
raisindb create function greet --lang assemblyscript --ns demo
raisindb function build wasm/demo/greet
raisindb function run   wasm/demo/greet --input '{"name":"Ada"}'
```

## Writing a handler

```ts
import { run, log, nodes, unknownHandler, cabi_realloc } from "@raisindb/function-assemblyscript";

function greet(input: string): string {
  log.info("greeting");
  const children = nodes.getChildren("content", "/pages", 50);
  return '{"greeting":"hello","pages":' + children.length.toString() + "}";
}

// The component exports ONE function; the node's `entry_file` suffix picks the
// handler, so routing is an ordinary switch.
function route(name: string, input: string): string {
  if (name == "default") return greet(input);
  return unknownHandler(name, "default");
}

export function handler(np: i32, nl: i32, ip: i32, il: i32): i32 {
  return run(np, nl, ip, il, route);
}
export { cabi_realloc };
```

`handler` and `cabi_realloc` must both be exported from the entry module:
`wasm-tools component new` looks them up by name.

## Why the lowering is hand-written

AssemblyScript deliberately does not implement WASI or the Component Model and
has no `wit-bindgen` backend, so `asc` produces a core module while the host
requires a component. `assembly/abi.ts` is the bridge — the only file that
knows about pointers — and `raisindb function build` runs
`asc` → `wasm-tools component embed` → `wasm-tools component new`.

Two ABI details it exists to get right, both of which fail silently:

* An imported interface is named with its package version,
  `raisin:function/host@0.1.0`.
* A variant discriminant is a `u8` padded to the payload's alignment, so
  `result<string, string>` is `{ u8 tag, 3 pad, i32 ptr, i32 len }`. Read as an
  `i32` the tag picks up padding and every `Ok` looks like an `Err` — with the
  payload still decoding correctly.

## Strings, not objects

Handlers take and return JSON **strings**. AssemblyScript has no built-in JSON,
and bundling one would make every artifact pay for it. Use
[`json-as`](https://github.com/JairusSW/as-json) if you want typed
(de)serialisation, or build strings directly for simple outputs.
