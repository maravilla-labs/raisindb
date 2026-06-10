# Ecommerce Order Fulfillment Workflow

A complete, runnable real-use-case demo: fulfilling an ecommerce order with
RaisinDB workflows. It showcases the **SAGA pattern with TWO compensations
(verified LIFO rollback)**, a **fraud gate** routing to a human review, and
a **cancel path** that voids the charge and skips fulfillment:

- **JS functions deployed as nodes** (`raisin:Function` + child `index.js`
  asset with the source in the `code` property) under `/lib/ecommerce/`
- A **designer-format flow** (`/flows/fulfill-order`) with templated
  arguments (including an **array** passed through `${input.items}`),
  **two saga compensations** with `compensation_input_mapping`, an **OR
  container** fraud gate, and a **routing gate** on the human response
- The **inbox API** to find and complete the fraud review, resuming the flow

## Scenario

A customer places an order (`order_id`, `total`, `items[]`, `address`):

1. `charge_payment` charges the card. Saga compensation: `refund-payment`
   with `{ charge_id: ${output.charge_id} }`.
2. A fraud gate: orders **over 1000 CHF** or with **`flagged: true`** must
   be reviewed by `/users/admin` via the inbox (**release** / **cancel**).
   Everything else passes straight through (no rule matches → the OR
   container is skipped).
3. A routing gate on the review response:
   - **cancel** → `cancel_refund` voids the charge (forward use of
     `refund-payment`); fulfillment is skipped; the flow completes.
   - otherwise (released, or no review happened) → the `fulfill`
     AND-container runs:
     - `allocate_stock` reserves warehouse stock for the items. Saga
       compensation: `release-stock` with
       `{ allocation_id: ${output.allocation_id} }`.
     - `ship_order` books the carrier. If it fails permanently (scenario C
       simulates a carrier outage), the flow **rolls back**: BOTH
       compensations run in **LIFO order** — `release-stock` first, then
       `refund-payment` — each with inputs mapped from its forward step's
       output.

## Workflow diagram

```
            input: { order_id, total, items[], address, flagged?, simulate_carrier_outage? }
                                      │
                                      ▼
                  ┌─────────────────────────────────────┐
                  │ charge_payment  (function step)     │
                  │ /lib/ecommerce/charge-payment       │
                  │ compensation: refund-payment        │
                  │   { charge_id: ${output.charge_id} }│
                  └──────────────────┬──────────────────┘
                                     │ steps.charge_payment.charge_id
                                     ▼
          ┌──────────────────────────────────────────────────┐
          │ fraud_gate  (OR container, REL rules)            │
          │   rule 1: input.total > 1000          ──┐        │
          │   rule 2: input.flagged == true       ──┤        │
          │                                         ▼        │
          │              ┌───────────────────────────┐       │
          │              │ fraud_review (human task) │       │
          │              │ assignee /users/admin     │       │
          │              │ [Release] [Cancel]        │       │
          │              └─────────────┬─────────────┘       │
          │   no rule matches: skipped ┤                     │
          └────────────────────────────┼─────────────────────┘
                                       ▼
          ┌──────────────────────────────────────────────────┐
          │ routing_gate  (OR container)                     │
          │   rule 1: steps.fraud_review.action == "cancel"  │
          │           ──► cancel_refund (refund-payment,     │
          │               charge voided, fulfillment SKIPPED)│
          │   rule 2: true                                   │
          │           ──► fulfill (AND container):           │
          │      ┌─────────────────────────────────────┐     │
          │      │ allocate_stock (function step)      │     │
          │      │ items = ${input.items}  (array!)    │     │
          │      │ compensation: release-stock         │     │
          │      │  { allocation_id:                   │     │
          │      │    ${output.allocation_id} }        │     │
          │      └──────────────────┬──────────────────┘     │
          │      ┌──────────────────▼──────────────────┐     │
          │      │ ship_order (function step)          │     │
          │      │ retry_strategy none, timeout 120s   │     │
          │      │ fail ──► SAGA ROLLBACK (LIFO):      │     │
          │      │   1. release-stock                  │     │
          │      │   2. refund-payment                 │     │
          │      └─────────────────────────────────────┘     │
          └──────────────────────────────────────────────────┘
```

## Files

| File | Purpose |
|---|---|
| `run.mjs` | Deploys everything idempotently, then runs + asserts all four scenarios |
| `functions/charge-payment.js` | Charge the card, return `charge_id` |
| `functions/allocate-stock.js` | Reserve stock; validates the `items` ARRAY survived templates |
| `functions/ship-order.js` | Book the carrier; `fail: true` simulates an outage |
| `functions/release-stock.js` | Saga compensation for allocate_stock (LIFO **first**) |
| `functions/refund-payment.js` | Saga compensation for charge_payment (LIFO **last**) + forward cancel-path step |

## How to run

Start a dev-mode server, then:

```bash
# from the repo root
cargo build --package raisin-server --features "storage-rocksdb,websocket,pgwire"
./target/debug/raisin-server --config <your-config.toml> --dev-mode &

cd examples/workflows/ecommerce-order
npm install
RAISIN_URL=http://localhost:8081 node run.mjs
```

Environment variables (all optional): `RAISIN_URL`
(default `http://localhost:8081`), `RAISIN_REPO` (default `ecommerce-demo`),
`RAISIN_USER` / `RAISIN_PASSWORD` (default `admin` / `Admin12345!@#`).

The script is **idempotent**: it creates the repository, folders, function
nodes, and flow node only if missing, and refreshes their definitions when
they already exist - safe to run repeatedly against the same server.

### What gets verified

- Each function executes for real before any flow runs (a smoke invocation
  of `charge-payment` during setup; see pitfall 1 below for the endpoint).
- **Scenario A** - normal order (99 CHF, 2 line items): completes
  **without** review; the `charge_id` flowed into `allocate_stock`; the
  `items` **array survived the `${input.items}` template** (content
  round-trips); a `TRK-` tracking number references the allocation.
- **Scenario B** - high-value order (2500 CHF): the flow **waits**, the
  review task appears in `/users/admin`'s inbox, **release** resumes the
  flow, and the order ships.
- **Scenario C** - simulated carrier outage on a normal order: after the
  job system exhausts its retries (~40s) the flow status becomes
  `rolled_back` with **both** compensations `executed` and correctly mapped
  inputs (`allocation_id` / `charge_id` from each forward step's output).
  **LIFO ordering is proven live**: the engine persists the instance after
  each compensation, and the demo polls the instance node during rollback
  until it observes `release-stock` **executed while `refund-payment` is
  still pending** (and asserts the reverse state never appears). The
  compensation functions take ~2s each to keep that window wide.
- **Scenario D** - flagged order (450 CHF, `flagged: true`): review →
  **cancel**. The flow completes via the cancel path:
  `__human_response.action == "cancel"`, the response is also the
  `fraud_review` step output, `allocate_stock`/`ship_order` **never ran**,
  and `cancel_refund` voided the original charge.

The script exits non-zero on any assertion failure.

## Engine pitfalls this example works around (verified live)

Pitfalls 1-4 were first documented in the `event-ticketing` example and
reproduce here; 5-6 are additional findings from this example. The
workarounds are annotated inline in `run.mjs` / `functions/*.js`:

1. **`/api/functions/{repo}/...` ignores the caller's auth context.**
   `find_function_node` (`crates/raisin-transport-http/src/handlers/functions/helpers.rs:39`)
   builds its NodeService with `auth: None`; RLS then denies every node, so
   list/get/**invoke** return `[]`/404 even for admins. Workaround:
   smoke-test code execution via `POST /api/files/{repo}/run` (which does
   pass the auth context). Flow execution is unaffected.
2. **Fast functions lose their flow resume.** If a flow-invoked function
   finishes before the flow start/resume job has persisted the `waiting`
   state, the resume is silently dropped and the flow hangs until its wait
   deadline fails it. Workaround: the demo functions busy-wait >= 250 ms.
   Note `await new Promise(r => setTimeout(r, ...))` is NOT usable for
   this: it deadlocks in the QuickJS runtime ("blocking on a promise
   resulted in a dead lock").
3. **Queued jobs can be dropped at pickup** (`ERROR ... Missing job
   context`): a worker can grab a job before its JobDataStore context is
   written; the job is then dropped without retry. Observed dropping the
   inbox-completion resume job (flow stuck `waiting` forever). Workaround:
   if the flow is still `waiting` ~10s after task completion, re-issue the
   resume via `POST /api/flows/{repo}/instances/{id}/resume` with the same
   response payload (see `completeTaskAndWait` in `run.mjs`).
4. **Two independent retry layers on function steps.** The function
   execution *job* retries 3 times (~10s/30s fixed backoff) regardless of
   the step's retry config, and *then* the flow applies its own step
   retries (default 3) unless `retry_strategy: 'none'`. A step's
   `timeout_ms` (the flow wait deadline) must outlive the job retry
   schedule (~40s+), otherwise the wait times out first, the flow fails via
   timeout, and **saga compensation never runs** (`ship_order` uses
   `retry_strategy: 'none'` + `timeout_ms: 120000`).
5. **NEW - hyphenated step ids are unusable in REL expressions.** REL
   identifiers are `[A-Za-z_][A-Za-z0-9_]*`, so
   `steps.charge-payment.charge_id` parses as `steps.charge` **minus**
   `payment.charge_id` (→ "undefined variable: payment"). Bracket access
   (`steps["charge-payment"]`) parses, but unlike dot access it is **not
   null-safe** - it errors with "property not found" when the step has not
   run yet, which breaks `or`-container rules that must evaluate before /
   without that step (e.g. routing on an optional human task's response).
   Workaround: use **snake_case step ids** (`charge_payment`); dot access
   on missing steps is null-safe (`steps.fraud_review.action` → `null`).
6. **NEW - no per-entry execution timestamp in `compensation_stack`.**
   `CompensationEntry` records `completed_at` (of the FORWARD step) and a
   final status, but nothing that proves the *order* compensations ran in.
   To verify LIFO live, exploit that the engine saves the instance after
   **each** compensation (`runtime/compensation.rs`): poll the instance
   node during rollback and catch the intermediate state (later step's
   compensation `executed`, earlier step's still `pending`). Give
   compensation functions a couple of seconds of latency so the window is
   reliably observable.

Also inherited from the ticketing example: `raisin:Asset` requires a
`file` property even when the source is inline in `code` (use `file: ''`),
and `compensation_input_mapping` IS honored in designer format (the
docs/workflows.md §5.3 "designer-format gap" note is outdated).

## How functions are stored

A function is a `raisin:Function` node in the `functions` workspace whose
properties carry the metadata (`name`, `language: javascript`,
`entry_file: "index.js:handler"`, `enabled`, ...). The source code lives in
a **child `raisin:Asset` node** named like the entry file (`index.js`) with
the source in its inline `code` string property (the Functions IDE and the
package installer alternatively store it in binary storage via a `file`
resource property - the engine accepts both, see
`crates/raisin-functions/src/execution/code_loader.rs`).
