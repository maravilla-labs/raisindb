# Employee Onboarding Workflow

A complete, runnable real-use-case demo: onboarding a new employee with
RaisinDB workflows. It combines the main engine features in one flow:

- **JS functions deployed as nodes** (`raisin:Function` + child `index.js`
  asset with the source in the `code` property) under `/lib/onboarding/`
- A **designer-format flow** (`/flows/onboard-employee`) with templated
  arguments, **saga compensation**, an **OR container** routing on a REL
  rule, and a **mandatory human approval task**
- The **inbox API** to find and complete the approval, resuming the flow
- A downstream step (**send-welcome**) that consumes both an earlier step's
  output (`steps['create-accounts'].email`) and the human decision
  (`__human_response.action`)

## Scenario

A new hire (`name`, `role`, `start_date`) joins the company:

1. `create-accounts` provisions the email + system accounts (engineers get
   `github` + `vpn` on top of the base set). If any later step fails
   unrecoverably, the saga compensation `deprovision-accounts` disables the
   accounts again automatically.
2. An equipment gate: **engineers** get a laptop ordered
   (`order-laptop`); everyone else skips the container (no rule matches).
3. `manager-approval`: a human approval task for `/users/admin` in the
   inbox - the flow pauses until it is approved or rejected.
4. `send-welcome` composes the welcome email from the provisioned address
   and the manager's decision. A **rejection still completes the flow** -
   the decision is data, and the step branches to a rejection-notice text.

## Workflow diagram

```
                 input: { name, role, start_date }
                                │
                                ▼
              ┌──────────────────────────────────────┐
              │  create-accounts  (function step)    │
              │  /lib/onboarding/create-accounts     │
              │  compensation:                       │
              │   deprovision-accounts               │
              │   { account_id: ${output.account_id} │
              └──────────────────┬───────────────────┘
                                 │ steps['create-accounts'].{account_id,email}
                                 ▼
        ┌─────────────────────────────────────────────────┐
        │ equipment-gate  (OR container, REL rule)        │
        │                                                 │
        │  rule: input.role == "engineer"  ───────────┐   │
        │                                             ▼   │
        │                      ┌───────────────────────┐  │
        │                      │ order-laptop          │  │
        │                      │ (function step)       │  │
        │                      └───────────┬───────────┘  │
        │  no rule matches:                │              │
        │  container skipped ──────────────┤              │
        └──────────────────────────────────┼──────────────┘
                                           ▼
              ┌──────────────────────────────────────┐
              │  manager-approval  (human task)      │
              │  assignee /users/admin, priority 3   │
              │  [Approve] [Reject]                  │
              └──────────────────┬───────────────────┘
                                 │ __human_response.action
                                 ▼
              ┌──────────────────────────────────────┐
              │  send-welcome  (function step)       │
              │  email    = steps['create-accounts'] │
              │             .email                   │
              │  decision = __human_response.action  │
              │  approve -> welcome text             │
              │  reject  -> rejection notice         │
              └──────────────────┬───────────────────┘
                                 ▼
                 output: { welcome_text, sent, ... }
```

## Files

| File | Purpose |
|---|---|
| `run.mjs` | Deploys everything idempotently, then runs + asserts all four scenarios |
| `functions/create-accounts.js` | Provision email + system accounts (engineers get extra systems) |
| `functions/order-laptop.js` | Order hardware for engineers (only inside the equipment gate) |
| `functions/send-welcome.js` | Compose the welcome/rejection email from email + decision |
| `functions/deprovision-accounts.js` | Saga compensation: revoke the provisioned accounts |

## How to run

Start a dev-mode server, then:

```bash
# from the repo root
cargo build --package raisin-server --features "storage-rocksdb,websocket,pgwire"
./target/debug/raisin-server --config <your-config.toml> --dev-mode &

cd examples/workflows/employee-onboarding
npm install
RAISIN_URL=http://localhost:8081 node run.mjs
```

Environment variables (all optional): `RAISIN_URL`
(default `http://localhost:8081`), `RAISIN_REPO` (default `onboarding-demo`),
`RAISIN_USER` / `RAISIN_PASSWORD` (default `admin` / `Admin12345!@#`).

The script is **idempotent**: it creates the repository, folders, function
nodes, and flow node only if missing, and refreshes their definitions when
they already exist - safe to run repeatedly against the same server.

### What gets verified

- Each function executes for real before any flow runs (a smoke invocation
  of `create-accounts` during setup; see pitfall 1 below for the endpoint).
- **Scenario A** - engineer hire: the equipment gate routes to
  `order-laptop` (order references the provisioned `account_id`), the flow
  pauses on the approval, approving via `inbox.completeTask` resumes it,
  and the welcome text contains the provisioned email address.
- **Scenario B** - contractor hire: the equipment gate is **skipped** (no
  `order-laptop` output recorded), approval + welcome email as in A, and
  the account has no engineer-only systems.
- **Scenario C** - rejection: the manager rejects the task; the flow
  **still completes**, `__human_response.action == "reject"` is visible to
  `send-welcome` (passed as the `decision` argument), and the output is the
  rejection-notice variant with `sent: false`.
- **Scenario D** - simulated mail-gateway outage in `send-welcome`
  (`retry_strategy: 'none'`): after the job system exhausts its retries
  (~40s) the flow status becomes `rolled_back` and the instance node's
  `compensation_stack` shows `deprovision-accounts` as `executed` with
  `{ account_id }` mapped from the create-accounts output via
  `compensation_input_mapping`. (Compensation only pushes onto the saga
  stack when a step **succeeds** - that's why the rollback demo fails a
  *later* step, not `create-accounts` itself.)

The script exits non-zero on any assertion failure.

## Engine pitfalls this example works around (verified live)

All hit against `raisin-server` (main @ a8107fd, 2026-06-10); the
workarounds are annotated inline in `run.mjs` / `functions/*.js`.
Pitfalls 1-5 are the same ones documented in
`examples/workflows/event-ticketing/README.md`; 6 and 7 are new here:

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
   resume job (flow stuck `waiting` forever). Workaround: if the
   `manager-approval` step output hasn't been recorded ~10s after task
   completion, re-issue the resume via
   `POST /api/flows/{repo}/instances/{id}/resume` with the same response
   payload (see `approveTask` in `run.mjs`).
4. **Two independent retry layers on function steps.** The function
   execution *job* retries 3 times (~10s/30s fixed backoff) regardless of
   the step's retry config, and *then* the flow applies its own step
   retries (default 3) unless `retry_strategy: 'none'`. Consequence: a
   step's `timeout_ms` (the flow wait deadline) must outlive the job retry
   schedule (~40s+), otherwise the wait times out first, the flow fails via
   timeout, and **saga compensation never runs**.
5. **`raisin:Asset` requires a `file` property** even when the source is
   stored inline in `code` - create code assets with `file: ''`.
6. **(NEW) Hyphenated step ids need bracket access in expressions.** REL
   identifiers only allow `[A-Za-z0-9_]`, so
   `${steps.create-accounts.email}` parses as a *subtraction*
   (`steps.create - accounts.email`). Reference hyphenated step outputs
   with index access instead: `${steps['create-accounts'].email}` (also
   inside `{{ ... }}` interpolations). Plain hyphen-free paths like
   `__human_response.action` are unaffected.
7. **(NEW) `failed` is a transient status during a saga rollback.** Between
   the final step failure and the end of compensation execution the
   instance status reads `failed`; only afterwards does it flip to
   `rolled_back`. A poller asserting on the first terminal-looking status
   can race this window (observed live: ~1-2s). Workaround: treat `failed`
   as "rollback may be in progress" and keep polling for `rolled_back` for
   a grace period (see Scenario D in `run.mjs`).

## How functions are stored

A function is a `raisin:Function` node in the `functions` workspace whose
properties carry the metadata (`name`, `language: javascript`,
`entry_file: "index.js:handler"`, `enabled`, ...). The source code lives in
a **child `raisin:Asset` node** named like the entry file (`index.js`) with
the source in its inline `code` string property (the Functions IDE and the
package installer alternatively store it in binary storage via a `file`
resource property - the engine accepts both, see
`crates/raisin-functions/src/execution/code_loader.rs`).
