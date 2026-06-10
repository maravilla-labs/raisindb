# Picasso/MTeX Quote → Order → Supplier Workflow

A complete, runnable real-use-case demo implementing the MTeX project's
BPMN swimlane ("Quote → Order → Supplier Workflow") as a RaisinDB
workflow. It focuses on the engine features the event-ticketing example
does not cover:

- **THREE chained human tasks in one flow** - two approval tasks and one
  **input** task (structured JSON via `input_schema`), i.e. three full
  wait → inbox → resume cycles in a single instance
- A **decline gate** (OR container) that ends the flow cleanly when the
  quote is declined - the rest of the pipeline (including the second
  human task) is never created
- A **privacy/redaction function**: the supplier order email is prepared
  from the full customer record but must not leak any of it
  ("Supplier never sees customer name/contact/PO" - the core MTeX rule),
  with a mechanical `redaction_check` self-audit asserted by the demo

## The business scenario

A customer (with an account tier) requests a quote for UV-printing
products. MTeX checks feasibility and pricing, a human decides whether to
send the quote, the customer orders, MTeX approves the order, a human
picks the supplier, and the supplier blind drop-ships - without ever
learning who the end customer is.

## BPMN swimlane (source: homepage-mtex/workflow-swimlane.svg)

```
            │← ─ ─ PHASE 1: QUOTE ─ ─ →│← ─ PHASE 2: ORDER ─ →│← ─ ─ ─ PHASE 3: SUPPLIER ─ ─ ─ →│
            │                          │                      │                                 │
 CUSTOMER   │ Requests Quote   Receives Quote    Inputs Order │                       Sees Status:
            │ (Account Type)   (Pricing+Terms)   Form (tier-  │                       Pending → In
            │      │           Accepts Quote     specific)    │                       Transit → Dlvd
            │      ▼                ▲               │         │                            ▲
 ───────────┼──────┼────────────────┼───────────────┼─────────┼────────────────────────────┼─────
            │      ▼                │               ▼         │                            │
 MTeX TEAM  │ Reviews Request ──► Sends Quote   Receives Order│  Select Supplier   Prepares Order
            │ Checks feasibility               Reviews Details│  (HITL Decision) ► Email (Items,
            │                                  Approves Order │                    Address only -
            │                                  (Feasibility)  │                    NO cust. details)
 ───────────┼──────────────────────────────────────────────── ┼─────────────────────────│─────────
            │                                                 │                         ▼
 SUPPLIER   │                                                 │   Receives Order Email ──► Ships to
            │                                                 │   (NO customer details)    MTeX or
            │                                                 │                            Direct
            │
            │  "Supplier never sees customer name/contact/PO • No payment check required •
            │   MTeX reviews each order before supplier contact"
```

## BPMN element → RaisinDB step mapping

| BPMN element (lane) | RaisinDB step | Type |
|---|---|---|
| Requests Quote (CUSTOMER) | `flows.run('/flows/quote-to-order', { product, quantity, tier, customer })` | flow trigger (input) |
| Reviews Request / Checks feasibility (MTeX) | `check-feasibility` | function step → `/lib/picasso/check-feasibility` |
| Sends Quote ↔ Receives Quote + Accepts Quote | `quote-review` | human task (**approval**, send / decline) |
| Inputs Order Form → Receives Order / Reviews Details / Approves Order (Feasibility check) | `order-approval` | human task (**approval**, P4) |
| Quote declined → no order | `quote-gate` | OR container, rule `steps["quote-review"].action == "send"` |
| Select Supplier (HITL Decision) | `select-supplier` | human task (**input**, `input_schema` {supplier, shipping_mode}) |
| Prepares Order Email (NO customer details) | `prepare-supplier-email` | function step → redacts customer, self-audits |
| Ships to MTeX or Direct / Updates Status (Pending → In Transit) | `mark-shipped` | function step → `status: in_transit` + tracking ref |

## Flow shape (designer format, as deployed)

```
        input: { product, quantity, tier, customer{company,contact_name,email,po_number} }
                                   │
                                   ▼
                  ┌────────────────────────────────┐
                  │ check-feasibility (function)   │   unit_price, total_price, feasible
                  └────────────────┬───────────────┘
                                   ▼
                  ┌────────────────────────────────┐
                  │ quote-review (human approval)  │   WAIT #1  [Send quote] [Decline]
                  └────────────────┬───────────────┘
                                   ▼
        ┌──────────────────────────────────────────────────────────┐
        │ quote-gate (OR container)                                │
        │   rule: steps["quote-review"].action == "send"           │
        │   no match (declined) ──► container skipped ──► END      │
        │                                                          │
        │   ┌─ order-pipeline (AND container) ───────────────────┐ │
        │   │  order-approval        human approval   WAIT #2    │ │
        │   │  select-supplier       human INPUT task WAIT #3    │ │
        │   │    input_schema: { supplier: string,               │ │
        │   │                    shipping_mode: direct|via_mtex }│ │
        │   │  prepare-supplier-email  function (REDACTION)      │ │
        │   │  mark-shipped            function (in_transit)     │ │
        │   └────────────────────────────────────────────────────┘ │
        └──────────────────────────────────────────────────────────┘
```

## Files

| File | Purpose |
|---|---|
| `run.mjs` | Deploys everything idempotently, then runs + asserts all three scenarios |
| `functions/check-feasibility.js` | Quote pricing: catalog price × tier multiplier (starter 1.0 / business 0.95 / enterprise 0.9) |
| `functions/prepare-supplier-email.js` | Blind drop-ship email; redacts the customer, returns `redaction_check` self-audit |
| `functions/mark-shipped.js` | Status update Pending → In Transit with a tracking ref |

## How to run

Start a dev-mode server, then:

```bash
# from the repo root
cargo build --package raisin-server --features "storage-rocksdb,websocket,pgwire"
./target/debug/raisin-server --config <your-config.toml> --dev-mode &

cd examples/workflows/picasso-order
npm install
RAISIN_URL=http://localhost:8081 node run.mjs
```

Environment variables (all optional): `RAISIN_URL` (default
`http://localhost:8081`), `RAISIN_REPO` (default `picasso-demo`),
`RAISIN_USER` / `RAISIN_PASSWORD` (default `admin` / `Admin12345!@#`).

The script is **idempotent**: it creates the repository, folders,
function nodes, and flow node only if missing, and refreshes their
definitions when they already exist - safe to run repeatedly.

### What gets verified

- Functions execute for real before any flow runs (smoke invocations of
  `check-feasibility` and `prepare-supplier-email` during setup).
- **Scenario A - happy path through all three waits**: quote-review
  "send" → order-approval "approve" → select-supplier
  `{supplier: "UV-Print AG", shipping_mode: "via_mtex"}` → completed.
  Asserts: pricing (25 × 76 = 1900 CHF), each wait paused the flow and
  produced exactly the expected inbox task (order-approval is P4, the
  input task carries its `input_schema`), the supplier name appears in
  the email body while customer company / contact / email / PO do
  **not**, `redaction_check` is all-false, final status `in_transit`
  with a tracking ref, and no pending task remains.
- **Scenario B - quote declined**: `__human_response.action ==
  "decline"` is recorded, the flow **completes** without running any
  order-pipeline step, and **no second inbox task was ever created**
  (checked across pending AND completed tasks).
- **Scenario C - input-task round-trip**: the submitted JSON comes back
  **verbatim** in `steps["select-supplier"]` and `__human_response`
  (plus exactly two engine metadata fields: `completed_by`,
  `task_path`), and the choice drives the downstream email + carrier.

The script exits non-zero on any assertion failure.

## Engine pitfalls this example works around (verified live)

Items 1-4 were first documented in the event-ticketing example and
reproduce here; items 5-8 are NEW findings from this demo (chained human
tasks + input task type). All hit against `raisin-server` (main @
a8107fd, 2026-06-10); workarounds are annotated inline in `run.mjs`.

1. **`/api/functions/{repo}/...` ignores the caller's auth context** -
   RLS denies every function node so list/get/invoke return `[]`/404
   even for admins. Workaround: smoke-test via `POST
   /api/files/{repo}/run`. Flow execution is unaffected.
2. **Fast functions lose their flow resume** - demo functions busy-wait
   ~250 ms (`await setTimeout` is NOT usable: it deadlocks in QuickJS).
3. **Queued jobs can be dropped at pickup** ("Missing job context"):
   the job is visible to workers before its context is written; the
   loser is dropped without retry. See pitfall 6 for what this means
   with multiple waits per flow.
4. **Step `timeout_ms` must outlive the job retry schedule** (~40s+),
   otherwise the wait expires before the retries finish.
5. **NEW - hyphenated step ids need bracket indexing in REL.**
   raisin-rel identifiers only allow `[A-Za-z0-9_]`, so
   `steps.check-feasibility.total_price` parses as a SUBTRACTION
   (`steps.check - feasibility.total_price`) and fails evaluation. Use
   `steps["check-feasibility"].total_price` in `{{ }}` templates,
   `${ }` expressions and container rule conditions alike.
6. **NEW - dropped resumes also hit FUNCTION steps, and differently.**
   With several waits per instance (3 human + 3 function waits here)
   pitfall 3 strikes more often, in two distinct shapes:
   - *human task*: flow stuck `waiting` at the completed step →
     re-issue `flows.resume(id, response)` - but ONLY after checking
     the instance node's `current_node_id` still equals the completed
     step; blindly re-resuming after the flow advanced would feed the
     stale payload to the NEXT human task's wait;
   - *function step*: the function job itself completed fine but the
     resume carrying its result was dropped; the flow waits at the
     function step forever. A `function_call` wait expects the resume
     payload in job-result shape `{ success, result }`
     (process_resume_data stores it verbatim as `__function_result`),
     so the recovery re-runs the idempotent function via
     `/api/files/{repo}/run` and resumes with `{ success: true, result }`.
   See `recoverIfStuck()` in `run.mjs`.
7. **NEW - inbox tasks are listable before they are completable** (two
   transient races on immediate completion):
   - "Inbox task '<id>' not found" - the list scan sees the task before
     get-by-id does;
   - "Invalid state transition from running to resumed" - the human
     task step creates the inbox task BEFORE the flow persists its
     `waiting` status, and `complete_task` validates the instance is
     Waiting.
   Both clear within milliseconds - retry the completion briefly
   (`completeTaskWithRetry()` in `run.mjs`).
8. **NEW (behavior, not a bug) - human task output = response object.**
   A completed task's response becomes the STEP OUTPUT
   (`steps["select-supplier"]`), enriched with exactly two metadata
   fields (`completed_by`, `task_path`). `__human_response` holds only
   the LAST completed task's response - with chained human tasks,
   assert per-step via `step_outputs`, not via `__human_response`.
   Also note: in dev mode `completed_by` records the system principal
   (`"system"`), not the login username.

## How the decline gate works

The engine has no "stop flow" option on approval tasks - a decline is
just a recorded response and the flow continues. To get BPMN-style
"declined quotes never become orders", the entire order pipeline lives
inside an OR container whose single rule requires
`steps["quote-review"].action == "send"`. On decline no rule matches,
the container is skipped, and the flow runs to `end` - so the
order-approval task is provably never created (Scenario B asserts this
across pending and completed tasks).

## How functions are stored

Same pattern as the event-ticketing example: a `raisin:Function` node in
the `functions` workspace (metadata: `language: javascript`,
`entry_file: "index.js:handler"`, `enabled`, ...) with the source in a
child `raisin:Asset` node named `index.js`, code in the inline `code`
property and `file: ''` (the `file` property is required even when the
source is inline).
