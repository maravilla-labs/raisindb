# Event Ticketing Workflow

A complete, runnable real-use-case demo: ordering tickets for an event with
RaisinDB workflows. It combines the main engine features in one flow:

- **JS functions deployed as nodes** (`raisin:Function` + child `index.js`
  asset with the source in the `code` property) under `/lib/ticketing/`
- A **designer-format flow** (`/flows/ticket-order`) with templated
  arguments, **saga compensation**, and an **OR container** routing on REL
  rules to a **human approval task**
- The **inbox API** to find and complete the approval, resuming the flow

## Scenario

A customer orders `quantity` seats of a `tier` (`vip` = 150 CHF/seat,
`standard` = 50 CHF/seat) for an event:

1. `reserve-seats` reserves the seats and computes the total price. If any
   later step fails unrecoverably, the saga compensation
   `cancel-reservation` releases the reservation automatically.
2. An approval gate: orders **over 500 CHF** or **any VIP order** must be
   approved by `/users/admin` via the inbox. Everything else passes
   straight through (no rule matches → the OR container is skipped).
3. `issue-tickets` issues one ticket per seat for the reservation. If it
   fails permanently (scenario C simulates an outage), the flow **rolls
   back**: the saga compensation cancels the reservation with the input
   mapped from the reserve step's output.

## Workflow diagram

```
                       input: { event_id, quantity, tier }
                                      │
                                      ▼
                      ┌───────────────────────────────┐
                      │  reserve  (function step)     │
                      │  /lib/ticketing/reserve-seats │
                      │  compensation:                │
                      │   cancel-reservation          │
                      │   { reservation_id:           │
                      │     ${output.reservation_id} }│
                      └───────────────┬───────────────┘
                                      │ steps.reserve.{reservation_id,total_price}
                                      ▼
              ┌───────────────────────────────────────────────┐
              │ approval-gate  (OR container, REL rules)      │
              │                                               │
              │  rule 1: steps.reserve.total_price > 500  ──┐ │
              │  rule 2: input.tier == "vip"              ──┤ │
              │                                             ▼ │
              │                     ┌───────────────────────┐ │
              │                     │ approve (human task)  │ │
              │                     │ assignee /users/admin │ │
              │                     │ [Approve] [Reject]    │ │
              │                     └───────────┬───────────┘ │
              │  no rule matches:               │             │
              │  container skipped ─────────────┤             │
              └─────────────────────────────────┼─────────────┘
                                                ▼
                      ┌───────────────────────────────┐
                      │  issue  (function step)       │
                      │  /lib/ticketing/issue-tickets │
                      │  reservation_id =             │
                      │   ${steps.reserve              │
                      │        .reservation_id}       │
                      └───────────────┬───────────────┘
                                      ▼
                        output: { ticket_ids: [...] }
```

## Files

| File | Purpose |
|---|---|
| `run.mjs` | Deploys everything idempotently, then runs + asserts all three scenarios |
| `functions/reserve-seats.js` | Reserve seats, compute price (vip 150 / standard 50 per seat) |
| `functions/issue-tickets.js` | Issue `quantity` ticket IDs for a reservation |
| `functions/cancel-reservation.js` | Saga compensation: release a reservation |

## How to run

Start a dev-mode server, then:

```bash
# from the repo root
cargo build --package raisin-server --features "storage-rocksdb,websocket,pgwire"
./target/debug/raisin-server --config <your-config.toml> --dev-mode &

cd examples/workflows/event-ticketing
npm install
RAISIN_URL=http://localhost:8081 node run.mjs
```

Environment variables (all optional): `RAISIN_URL`
(default `http://localhost:8081`), `RAISIN_REPO` (default `ticketing-demo`),
`RAISIN_USER` / `RAISIN_PASSWORD` (default `admin` / `Admin12345!@#`).

The script is **idempotent**: it creates the repository, folders, function
nodes, and flow node only if missing, and refreshes their definitions when
they already exist - safe to run repeatedly against the same server.

### What gets verified

- Each function executes for real before any flow runs (a smoke invocation
  of `reserve-seats` during setup; see pitfall 1 below for the endpoint).
- **Scenario A** - standard order (2x standard = 100 CHF): completes
  **without** pausing; `issue-tickets` ran (2 ticket IDs in the instance's
  step outputs); the approval step was skipped.
- **Scenario B** - VIP order (4x vip = 600 CHF): the flow **waits**, the
  approval task appears in `/users/admin`'s inbox, approving it resumes the
  flow, and 4 tickets are issued.
- **Scenario C** - simulated `issue-tickets` outage: after the job system
  exhausts its retries (~40s) the flow status becomes `rolled_back` and the
  instance node's `compensation_stack` shows `cancel-reservation` as
  `executed` with `{ reservation_id }` mapped from the reserve output via
  `compensation_input_mapping`.

The script exits non-zero on any assertion failure.

## Engine pitfalls this example works around (verified live)

All hit against `raisin-server` (main @ a8107fd, 2026-06-09); the
workarounds are annotated inline in `run.mjs` / `functions/*.js`:

1. **`/api/functions/{repo}/...` ignores the caller's auth context.**
   `find_function_node` (`crates/raisin-transport-http/src/handlers/functions/helpers.rs:39`)
   builds its NodeService with `auth: None`; RLS then denies every node
   ("RLS: No auth context set" in the log), so list/get/**invoke** return
   `[]`/404 even for admins. Workaround: smoke-test code execution via
   `POST /api/files/{repo}/run` (which does pass the auth context). Flow
   execution is unaffected - the flow runtime loads functions via storage
   directly, bypassing the RLS-filtered service.
2. **Fast functions lose their flow resume.** If a flow-invoked function
   finishes before the flow start/resume job has persisted the `waiting`
   state, the resume hits "Cannot resume flow - still in pending state" /
   "Invalid state transition from pending to running" and is silently
   dropped; the flow hangs until its wait deadline fails it. Workaround:
   the demo functions busy-wait ~250 ms ("external system latency"). Note
   `await new Promise(r => setTimeout(r, ...))` is NOT usable for this: it
   deadlocks in the QuickJS runtime ("blocking on a promise resulted in a
   dead lock").
3. **Queued jobs can be dropped at pickup** (`ERROR ... Missing job
   context`): `register_job()` makes a job visible to workers before
   `JobDataStore.put()` writes its context; when a worker wins that race
   the job is dropped without retry. Observed dropping the inbox-approval
   resume job (flow stuck `waiting` forever). Workaround: if the flow is
   still `waiting` ~10s after task completion, re-issue the resume via
   `POST /api/flows/{repo}/instances/{id}/resume` with the same response
   payload.
4. **Two independent retry layers on function steps.** The function
   execution *job* retries 3 times (~10s/30s fixed backoff) regardless of
   the step's retry config, and *then* the flow applies its own step
   retries (default 3) unless `retry_strategy: 'none'`. Consequence: a
   step's `timeout_ms` (the flow wait deadline) must outlive the job retry
   schedule (~40s+), otherwise the wait times out first, the flow fails via
   timeout, and **saga compensation never runs**.
5. **`raisin:Asset` requires a `file` property** even when the source is
   stored inline in `code` - create code assets with `file: ''`.
6. (Docs drift, positive) `compensation_input_mapping` **is** honored in
   designer format now - `docs/workflows.md` Appendix B item 2 is outdated.

## How functions are stored

A function is a `raisin:Function` node in the `functions` workspace whose
properties carry the metadata (`name`, `language: javascript`,
`entry_file: "index.js:handler"`, `enabled`, ...). The source code lives in
a **child `raisin:Asset` node** named like the entry file (`index.js`) with
the source in its inline `code` string property (the Functions IDE and the
package installer alternatively store it in binary storage via a `file`
resource property - the engine accepts both, see
`crates/raisin-functions/src/execution/code_loader.rs`).
