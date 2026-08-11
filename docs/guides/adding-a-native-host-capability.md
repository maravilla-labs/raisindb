# Adding a Native Host Capability

**Audience:** RaisinDB contributors (Rust). This is the repeatable pattern for
exposing new Rust-native functionality to server-side functions in **both**
runtimes — QuickJS (JavaScript) and Starlark — from a single implementation.

The strategic goal: **users can write connectors to almost any service.** Most
services speak HTTP, so most connectors need no Rust at all — they call
`raisin.http.fetch` from a pure-JS/Starlark adapter. When a service speaks a
*protocol* that HTTP cannot carry (a raw TCP/TLS protocol like IMAP, SMTP,
Redis, a binary RPC), we add that capability **natively in Rust** and expose it
to functions as a new `raisin.<namespace>.*` API. The native
[`raisin.imap`](../reference/virtual-node-adapters.md#imap) binding is the first
worked example, and this guide walks through it end to end.

---

## 1. Decide: native binding vs pure-JS adapter

Ask one question: **does the service speak HTTP?**

| Situation | What to build | Rust needed? |
|-----------|---------------|--------------|
| Service has an HTTP/HTTPS API (REST, JMAP, GraphQL, gRPC-web…) | A pure-JS/Starlark adapter calling `raisin.http.fetch` | **No** |
| Service speaks a non-HTTP wire protocol (IMAP, SMTP, POP3, raw TCP, a DB wire protocol) | A **native binding** (`raisin.<ns>.*`) in Rust, then a thin adapter on top | **Yes** |

The sandbox's only network egress is `raisin.http.fetch` — there is **no raw TCP
socket** available to function code. That is deliberate (it keeps the egress
surface auditable and policy-gated). So a genuine new protocol cannot be a
pure-JS adapter; the protocol has to be implemented in Rust behind a host API.

The IMAP adapter (`builtin-packages/imap-adapter/`) originally spoke **JMAP over
HTTP** precisely because no raw IMAP socket existed. The native `raisin.imap`
binding closes that gap: Rust now owns the IMAP protocol (TLS + `LOGIN` + `UID
FETCH`), and functions call high-level operations against a real IMAP server.

> **Rule of thumb:** reach for a native binding only when `raisin.http.fetch`
> genuinely cannot express the protocol. A new REST/JMAP/GraphQL provider is
> *always* a pure-JS adapter — do not add Rust for it.

---

## 2. Anatomy of a native binding (five pieces)

A new native namespace `raisin.<ns>` is four edits. Using `imap` as the
worked example, with the real paths from what landed:

| # | Piece | File(s) |
|---|-------|---------|
| 1 | **Protocol implementation** (pure Rust) | `crates/raisin-functions/src/runtime/imap/` (`mod.rs`, `client.rs`, `parse.rs`) |
| 2 | **`FunctionApi` trait methods** + real impl + **mock impl** | `api/traits.rs`, `api/raisindb/imap.rs`, `api/mock/mod.rs` |
| 3 | **Shared registry descriptors** (drives Starlark, python, typescript) | `runtime/bindings/methods/imap.rs`, registered in `methods/mod.rs` |
| 4 | **QuickJS ergonomic wrapper** (`raisin.<ns>.*`) | `runtime/quickjs/api_wrapper.js` |

Plus one edit outside the crate: the `namespace <ns>` block in
`packages/raisindb-functions-types/raisin.d.ts`.

All Rust paths are under `crates/raisin-functions/src/`. `http.rs`, `locks.rs`, and
`integrations.rs` are the templates for each layer; `integrations_sync_now` (added
alongside IMAP) is the freshest end-to-end example to grep and copy.

The mock impl in step 2 is **not optional** — the `FunctionApi` trait has no
default bodies, so omitting it fails the build.

The key idea: **one registry descriptor reaches every runtime.** Starlark
generates its namespace from the descriptor's `category` automatically; QuickJS
needs only a hand-written ergonomic wrapper over the same generic gateway. You
write the protocol once and both runtimes get it.

---

## 3. Step 1 — Implement the protocol in Rust

Put the actual protocol client in its own module, kept small (files < 300 lines).
For IMAP that is `crates/raisin-functions/src/runtime/imap/`:

- `mod.rs` — public types: `ImapConn` (the `{ host, port, tls, username, password }`
  descriptor), `MessageSummary`, `FetchSinceResult`, `MailboxInfo`, `MessageDetail`,
  and the `ImapError` enum (a `raisin-error`/`thiserror` type with a `code()` for
  stable error codes like `policy_denied`).
- `client.rs` — the async operations: `fetch_since`, `list_mailboxes`,
  `fetch_message`. These open the TLS connection and speak the protocol.
- `parse.rs` — RFC822 → structured (headers, from/to/subject/date, snippet), via
  the pure-Rust `mail-parser` crate.

Two conventions that are **not optional**:

- **Redact credentials in `Debug`.** `ImapConn` hand-implements `Debug` to print
  `password: "<redacted>"` (`imap/mod.rs`) so a stray `{:?}` or `tracing` render
  can never leak a secret. Do the same for any type holding a token/password.
- **Return a resumable cursor, never a null one.** `fetch_since` returns
  `{ messages, highestUid, uidvalidity }`; when nothing is new it returns the
  *unchanged* `highestUid`, and it surfaces `uidvalidity` so the adapter can detect
  a mailbox reset and force a full resync. (Mirrors the adapter contract's
  "never return `next_token: null`" rule.)

### Dependencies

Add protocol/parse crates to `crates/raisin-functions/Cargo.toml`. For IMAP:

```toml
async-imap = { version = "0.11", default-features = false, features = ["runtime-tokio"] }
mail-parser = "0.11"
tokio-rustls = "0.26"
```

`async-imap` drives IMAP over a tokio `AsyncRead`/`AsyncWrite`; we supply a
`tokio-rustls` TLS stream, reusing the workspace's existing rustls 0.23 /
tokio-rustls 0.26 stack — no new TLS major version. Prefer reusing a TLS stack the
workspace already pins over introducing another.

---

## 4. Step 2 — `FunctionApi` trait + real + mock impls

The `FunctionApi` trait (`crates/raisin-functions/src/api/traits.rs`) is the
runtime-agnostic seam every binding funnels through. Add one async method per
operation:

```rust
async fn imap_fetch_since(&self, conn: Value, since_uid: i64, opts: Option<Value>) -> Result<Value>;
async fn imap_list_mailboxes(&self, conn: Value) -> Result<Value>;
async fn imap_fetch_message(&self, conn: Value, uid: i64, opts: Option<Value>) -> Result<Value>;
```

Arguments and returns are `serde_json::Value` — that is the lingua franca both
runtimes marshal to/from.

**Real impl:** `api/raisindb/imap.rs` implements these on `RaisinFunctionApi`,
enforcing the network policy (Step 4 below) and delegating to the `runtime::imap`
client.

**Mock impl:** `api/mock/mod.rs` provides deterministic fixtures (a canned set of
messages, mailboxes) so binding tests and function-unit tests run without a live
server. Keep the mock in lockstep with the trait — an unimplemented trait method
won't compile the mock.

---

## 5. Step 3 — Shared registry (reaches Starlark + python/typescript)

`crates/raisin-functions/src/runtime/bindings/methods/imap.rs` exports
`pub fn methods() -> Vec<ApiMethodDescriptor>`. Each descriptor declares the
call's shape once, for *every* non-QuickJS runtime:

```rust
ApiMethodDescriptor {
    internal_name: "imap_fetch_since",
    js_name: "fetchSince",
    py_name: "fetch_since",
    category: "imap",
    args: vec![
        ArgSpec::new("conn", ArgType::Json),
        ArgSpec::new("sinceUid", ArgType::I64),
        ArgSpec::new("opts", ArgType::OptionalJson),
    ],
    return_type: ReturnType::Json,
    invoker: |api, args| Box::pin(async move {
        let mut parser = ArgParser::new(&args);
        let conn = parser.json()?;
        let since_uid = parser.i64()?;
        let opts = parser.optional_json()?;
        Ok(InvokeResult::Json(api.imap_fetch_since(conn, since_uid, opts).await?))
    }),
}
```

The `invoker` is the only glue: it parses positional args and calls the
`FunctionApi` method. The registry (`runtime/bindings/registry.rs`) uses
`js_name`/`py_name`/`args` to generate the Starlark (and python/typescript)
wrappers automatically — so `raisin.imap.fetchSince(...)` exists in Starlark the
moment the descriptor is registered.

Register the module in `runtime/bindings/methods/mod.rs`:

```rust
pub mod imap;
// ...
methods.extend(imap::methods());
```

(The `internal_name`s also appear in the method-name allow-list in `methods/mod.rs`.)

---

## 6. Step 4 — QuickJS wrapper (reaches JavaScript)

**There is no per-namespace QuickJS module any more.** QuickJS reaches every
registry method through ONE generic dispatcher,
`runtime/quickjs/gateway.rs`'s `__raisin_call(method, argsJson)`, registered once
in `environment.rs` via `register_registry_gateway`. Starlark uses the same
registry through `runtime/starlark/gateway.rs`, and *auto-generates* its
namespaces from each descriptor's `category` (`starlark/setup_code.rs`), so
**Starlark needs no per-namespace code at all** — only add your category to the
skip-list there if you want it excluded.

So the only QuickJS work is the ergonomic wrapper. Writing a
`runtime/quickjs/api_<ns>.rs` and registering it in `environment.rs` — as earlier
versions of this guide instructed — produces a file nothing calls;
`environment.rs` registers only `temp`, `fetch` and the gateways.

**Ergonomic wrapper** — add a `raisin.imap` block to
`runtime/quickjs/api_wrapper.js`, next to the `raisin.locks` / `raisin.integrations`
blocks. It calls the gateway helper `__call(internalName, args)` with your
descriptor's `internal_name`, and throws on `{ error }`:

```javascript
imap: {
    // fetchSince(conn, sinceUid, opts?) -> { messages, ... }
    fetchSince: (conn, sinceUid, opts) => {
        const r = __call('imap_fetch_since',
            [conn, sinceUid, opts === undefined ? null : opts]);
        if (r && r.error) throw new Error(r.message || r.error);
        return r;
    },
    // listMailboxes, fetchMessage …
},
```

`__call` handles the JSON encode/decode, so pass real values, not strings. Compare
the `raisin.locks` block in the same file for the established shape, including the
`__isErr(r) ? false : r` idiom where a failure should degrade rather than throw.

Now `raisin.imap.fetchSince(...)` exists in QuickJS too — same protocol code, same
`FunctionApi`, same policy gate.

**Also update `packages/raisindb-functions-types/raisin.d.ts`.** Despite its
"auto-generated" header it has **no generator and no drift test**, and
`packages/raisindb-skills` tells agents it is the authoritative API list — a
namespace missing from it is invisible to every TypeScript author and every
skill-driven agent, with no CI signal.

---

## 7. Security — network policy gating (do not skip)

A native protocol binding opens sockets, so it **must** honor the same egress
policy that gates `raisin.http.fetch`. Function egress is governed by the
function's `NetworkPolicy` (`crates/raisin-functions/src/types/config.rs`), whose
`is_url_allowed(&self, url) -> bool` glob-matches the `allowed_urls` from the
function's `.node.yaml`.

The rule: **build a synthetic URL for the connection and require the policy to
allow it, before opening any socket.** For IMAP, `ImapConn::policy_url()`
(`runtime/imap/mod.rs`) yields `imaps://{host}:{port}` (or `imap://` when
`tls=false`), and `authorize_imap` (`api/raisindb/imap.rs`) checks it up front:

```rust
fn authorize_imap(&self, conn: Value) -> Result<ImapConn> {
    let conn = ImapConn::from_value(&conn)?;
    let policy_url = conn.policy_url();
    if !self.is_url_allowed(&policy_url) {
        return Err(raisin_error::Error::PermissionDenied(format!(
            "[imap:policy_denied] IMAP endpoint not allowed by network policy: {policy_url}"
        )));
    }
    Ok(conn)
}
```

Every operation (`impl_imap_fetch_since`, `impl_imap_list_mailboxes`,
`impl_imap_fetch_message`) calls `authorize_imap` *first*, so a disallowed host can
never be contacted. An adapter authorizes IMAP by declaring, e.g.:

```yaml
network_policy:
  allowed_urls:
    - "imaps://imap.gmail.com:993"
```

A function with no matching pattern is refused with a permission error and never
opens a connection. There is a regression test for exactly this —
`imap_disallowed_host_refused_before_connect` in `api/raisindb/tests.rs` — asserting
that a host not on the allow-list is rejected **before** any socket is opened. Add
the equivalent test for any new binding.

**Never log credentials.** The connection secret (`conn.password`, which may be an
app password or an XOAUTH2 access token) must never appear in a log line, error
message, or trace field. Redact it at the type level (Step 1's `Debug` impl) and
keep it out of every error string. The `policy_url` is safe to log — it carries
only `host:port`, no secret.

---

## 8. The payoff

One protocol implementation, gated once, reaches both runtimes:

```
runtime/imap/ (protocol)  ─┐                          ┌─► starlark/gateway.rs  ──► Starlark (auto-generated
                           │                          │                             from the `category`)
                           ├─►  FunctionApi::imap_*  ──┤
                           │    via methods/imap.rs    │
NetworkPolicy gate ────────┘    (the ONE registry)     └─► quickjs/gateway.rs   ──► QuickJS, via the
                                                                                    api_wrapper.js block
```

That is the "users can write connectors to almost any service" story: HTTP
services need no Rust (`raisin.http.fetch` + a pure-JS adapter); a genuinely new
protocol gets a native binding added once, and every function runtime can use it.
The IMAP binding is the reference — grep `imap_fetch_since` (or
`integrations_sync_now`) to see the exact shape end to end, and copy it.

## See also

- [Virtual Node Adapters — Reference Contract](../reference/virtual-node-adapters.md)
  — the host APIs available to adapters, including `raisin.imap.*`.
- [Virtual Nodes — Engine Internals](../concepts/virtual-nodes-internals.md#native-protocol-bindings)
  — the "Native protocol bindings" note.
- [Building an Adapter](./building-an-adapter.md) — the adapter-author walkthrough.
