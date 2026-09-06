# AssemblyScript guest (proof of path)

AssemblyScript is not a supported guest language yet. This is the artifact that
proves it *can* be one, and the regression test that keeps the path open.

## Why it is hand-lowered

AssemblyScript deliberately stayed out of WASI and the Component Model, and
there is no maintained `wit-bindgen` backend for it, so `asc` emits a **core
module** while the host requires a **component**. Nothing off the shelf bridges
that.

What makes the gap crossable is that our WIT world is string-only. The
canonical ABI for it is small enough to write by hand:

* `handler: func(name: string, input: string) -> result<string, string>`
  flattens to `(i32 namePtr, i32 nameLen, i32 inputPtr, i32 inputLen)` and,
  because the result needs more than one flat value, returns a **pointer** to a
  return area laid out `{ i32 tag, i32 ptr, i32 len }` — tag 0 is ok, 1 is err.
* The guest exports `cabi_realloc` so the host can allocate into its memory.
  `wasm-tools component new` looks for it by name.

`src/guest.ts` implements exactly that and echoes both parameters back, so the
test can assert the LOWERING rather than just that something ran: a wrong
pointer or length corrupts a value instead of failing loudly.

## Regenerate

```bash
cd fixtures/wasm-guests/assemblyscript
npm install
npx asc src/guest.ts -o guest.core.wasm --runtime stub --exportRuntime --optimize --use abort=
wasm-tools component embed ../../../crates/raisin-functions/wit guest.core.wasm \
  -o guest.embedded.wasm --world function
wasm-tools component new guest.embedded.wasm -o guest.component.wasm
cp guest.component.wasm ../../../crates/raisin-functions/src/runtime/wasm/fixtures/assemblyscript.wasm
```

Verify with `wasm-tools component wit guest.component.wasm` — it must export
`handler: func(name: string, input: string) -> result<string, string>`.

Tooling used: AssemblyScript 0.28.20, wasm-tools 1.258.0.

## What is still missing for a real SDK

This proves the EXPORT side. A usable SDK also needs the IMPORT side — lowering
`call`, `log`, `context` and `abi-version` from `raisin:function/host`, which
means passing a caller-owned return area into each import and reading strings
back out of it. Same ABI, more of it. See `docs/design/assemblyscript-guest.md`.
