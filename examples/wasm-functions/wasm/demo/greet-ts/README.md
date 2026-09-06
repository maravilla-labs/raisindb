# greet-ts

A RaisinDB function as a WebAssembly component, built from an **unmodified
QuickJS-style handler**: `src/index.js` uses `globalThis.raisin` and `console`
exactly as a `language: javascript` function does. The only difference is that
the handlers are `export`ed (ESM), which is what lets one artifact carry two of
them.

```sh
npm install
npm test                     # native, against the mock host
npm run build                # jco componentize -> main.wasm (8-12 MB)
raisindb function build .    # the same thing, and copies it to the node dir
```

`tests/server.json` is the `raisindb function test --server` fixture: the same
inputs run against a real server, so the native and hosted answers must match.

What is not available in this runtime — `fetch`, timers, the `Resource` image
helpers — is listed in `sdks/ts/function-wasm/README.md`.
