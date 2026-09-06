# Cross-runtime benchmark sources

The JavaScript and Starlark arms of the runtime comparison, kept HERE rather
than inline in either test because two crates run them:

* `crates/raisin-functions/src/runtime/bench_runtimes.rs` — against the mock
  host, isolating runtime overhead.
* `crates/raisin-server/tests/all/runtime_bench_test.rs` — against a live
  server with real nodes and real SQL.

Both `include_str!` these files, so the arms cannot drift apart and start
timing different work. The WebAssembly arm is `../wasm-guests/bench`, whose
handlers mirror these function-for-function.

Every handler takes its workspace, path and SQL FROM ITS INPUT (with defaults),
which is what lets one source serve both a mock host and a real one.
