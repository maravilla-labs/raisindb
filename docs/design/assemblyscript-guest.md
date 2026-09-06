# AssemblyScript as a guest language

**Status:** export path PROVEN, import path not started. Written 2026-09-06.

## Why we want it

The TypeScript guest we shipped runs real JavaScript inside the component by
embedding a JS engine (ComponentizeJS/StarlingMonkey). That costs 8–15 MB per
artifact and buys isolation rather than speed — QuickJS already runs JavaScript
well, so the value is thin and we have stopped documenting it.

AssemblyScript is the shape we actually want for that slot: TypeScript-like
syntax, compiled ahead of time to a small WebAssembly module with no embedded
engine. Artifacts in the tens of kilobytes, Rust-class startup, and a language
our users already read.

## The problem: AssemblyScript does not speak the Component Model

This is the fact that decides the whole design, and it is not a temporary gap.
The AssemblyScript project deliberately stayed out of WASI and the Component
Model, on the grounds that interface types do not line up with JavaScript
semantics. The Bytecode Alliance's language-support page lists Rust, TinyGo,
JavaScript (jco), Python (componentize-py) and .NET. AssemblyScript is absent,
and there is no maintained `wit-bindgen` backend for it.

So `asc` gives us a **core module**. Our host requires a **component** exporting
the `raisin:function/function` world. Nothing off the shelf bridges that.

## The path that does work

Our world is unusually small, and every parameter and return value is a string:

```wit
call: func(method: string, args: string) -> result<string, string>;
log: func(level: log-level, message: string);
context: func() -> string;
abi-version: func() -> string;
export handler: func(name: string, input: string) -> result<string, string>;
```

That means we do not need a general bindings generator — we need the canonical
ABI for exactly one world, which is a bounded amount of hand-written code:

1. **Lowering, in AssemblyScript.** Strings cross as `(ptr, len)` UTF-8. The
   guest exports `cabi_realloc` so the host can allocate into guest memory, and
   returns `result<string, string>` through a return-area pointer. A few hundred
   lines, written once, shipped as the SDK.
2. **`wasm-tools component embed`** to attach the WIT to the core module, then
   **`wasm-tools component new`** to produce the component. Both are stable and
   already how every non-Rust language does this step.
3. **The existing pipeline is unchanged after that.** `raisin.build.yaml` gains
   a `lang: assemblyscript` arm whose command is `asc … && wasm-tools component
   embed … && wasm-tools component new …`; `function build`, `doctor`, `run`,
   package install and the server all see an ordinary component.

## The spike: done, and it passed

Run on 2026-09-06 with AssemblyScript 0.28.20 and wasm-tools 1.258.0:

1. A guest exporting `handler` with hand-written `(ptr, len)` lowering and
   `cabi_realloc` compiles with
   `asc --runtime stub --exportRuntime --optimize --use abort=` to a **1,540
   byte** core module.
2. `wasm-tools component embed wit … --world function` then
   `component new` produces a **1,856 byte** component. `wasm-tools component
   wit` confirms it exports
   `handler: func(name: string, input: string) -> result<string, string>`.
3. The existing host loads and runs it, and both string parameters arrive
   intact.

The artifact and its source are checked in at
`fixtures/wasm-guests/assemblyscript/`, with a regression test
(`an_assemblyscript_component_runs_and_receives_both_string_arguments`) so the
path cannot rot silently. For scale: 1.8 KB against 8–15 MB for the
ComponentizeJS guest this is meant to replace.

## What remains

The spike proves the EXPORT side. A usable SDK needs the IMPORT side too —
lowering `call`, `log`, `context` and `abi-version`, where the guest passes a
return area it owns into each import and reads strings back out. Same ABI, more
of it, and the part where mistakes are silent rather than loud. After that it is
ordinary SDK work: a `lang: assemblyscript` arm in `raisin.build.yaml`, a
template, and the existing `function build/doctor/run` pipeline unchanged.

## Risks

- **We would own a mini bindings generator.** If the WIT world grows past
  strings — a record, a list, a resource — the hand-written lowering has to grow
  with it. Keeping the world string-only is what makes this tractable, and is a
  reason not to "improve" the world into typed imports later.
- **AssemblyScript's stance may not change**, so this stays our code
  indefinitely rather than converging on an upstream generator.
- **No `wasi:cli` means no `stdout`.** Guest logging must go through the `log`
  import rather than `console.log`, unlike the other guests.

## Decision

Take the spike when the wasm work next gets attention. Until it passes, the
docs name AssemblyScript as intended and claim nothing more.
