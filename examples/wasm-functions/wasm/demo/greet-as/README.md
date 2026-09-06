# greet-as

An AssemblyScript RaisinDB function, compiled to a WebAssembly component.

```bash
npm install
raisindb function doctor .        # checks asc + wasm-tools
raisindb function build .         # asc -> component embed -> component new
raisindb function run . --input '{"name":"Ada"}'
```

## Why three build steps

AssemblyScript does not implement the Component Model, so `asc` produces a
CORE MODULE. `wasm-tools component embed` attaches the WIT in `wit/` and
`component new` wraps the result. `raisindb function build` runs all three.

## Handlers

`handler` and `cabi_realloc` must both be exported from `assembly/index.ts`
— they are resolved by name. Add a handler by extending `route` and creating a
Function node whose `entry_file` is `main.wasm:<name>`; both nodes then share
this one artifact.

## JSON

Handlers take and return JSON **text**. AssemblyScript has no built-in JSON and
bundling one would make every artifact pay for it; use
[`json-as`](https://github.com/JairusSW/as-json) if you want typed
(de)serialisation.
