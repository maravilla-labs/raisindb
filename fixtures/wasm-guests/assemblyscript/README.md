# AssemblyScript guest (proven, both directions)

AssemblyScript is not a supported guest language YET — there is no SDK and the
CLI will not scaffold one. This is the artifact proving the whole ABI works, and
the regression test that keeps it working.

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

## The import side, and the trap in it

`src/guest.ts` exercises all of it: `log`, `context` and `call`. Two details
cost real debugging time and are the reason this fixture exists:

**The interface name carries the package version.** `@external` must name
`raisin:function/host@0.1.0`, not `raisin:function/host`. Without the version
`wasm-tools component new` fails with "failed to resolve import", which at least
fails loudly.

**A variant discriminant is a `u8`, not an `i32`.** The canonical ABI stores a
tag in the smallest integer that fits the case count, then pads to the payload's
alignment, so `result<string, string>` is `{ u8 tag, 3 bytes pad, i32 ptr,
i32 len }`. Reading the tag with `load<i32>` picks up the padding and turns
every `Ok` into an `Err` — **while the payload still decodes perfectly**. That
is what a silent ABI bug looks like, and why the test asserts the tag and the
decoded body separately rather than just "it ran".

## What is still missing for a real SDK

The ABI is proven; what is missing is packaging it — typed wrappers generated
from the bindings registry (as the Rust and Go SDKs are), a `lang:
assemblyscript` arm in `raisin.build.yaml`, and a CLI template. No unknowns
remain, only work. See `docs/design/assemblyscript-guest.md`.
