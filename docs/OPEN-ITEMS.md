# RaisinDB — Open Items

Engine-owned gaps, triaged from downstream application feedback. Each entry
records the **symptom**, the **root cause** with file references, a
**verdict** (fix in the engine / solvable in the application / accept), and —
when it is not being fixed — the pattern that works instead.

The bar for "fix in the engine" is deliberate: a gap belongs here only if
closing it makes the engine more general without costing flexibility,
performance, or scalability, and without turning a generic multi-model
database into an application framework. A gap an application can close with
the primitives already shipped (functions, flows, triggers, the WebSocket
channel, SQL, RLS) stays with the application, and this file says so and
names the pattern.

When you close one, move it to the "Recently closed" section with what
changed, then delete it after a release.

---

## 1. Recently closed

### 1.1 Human-task deadlines and priority ignored templates

**Was:** a `human_task` step's `due_in_seconds` / `priority` were read as
raw integers *before* template resolution, so `due_in_seconds:
"${input.sla_seconds}"` resolved to a JSON string, failed `as_i64()`, and
was **silently dropped** — no `due_at`, no wait deadline, no `timeout_edge`,
no escalation. Only a flow-authored constant worked, which made per-run and
per-policy deadlines inexpressible. `Wait`'s own `duration`/`until` had
always resolved templates, so the inconsistency was invisible until someone
tried it.

**Fixed:** `build_task_properties`
(`crates/raisin-flow-runtime/src/handlers/human_task/handler.rs`) now
inserts both properties raw, lets `DataMapper::map` resolve them with
everything else, and coerces afterwards — accepting a number or a numeric
string. A value that resolves to something non-numeric is now a reported
configuration error instead of a silently missing deadline; an unresolvable
expression leaves the task with no deadline (the safe outcome), never a
bogus one. Tests:
`handlers::human_task::tests::test_human_task_resolves_templated_due_and_priority`
and the two neighbouring cases.

### 1.2 Human task types were a closed enum

**Was:** `task_type` was closed to `approval | input | review | action` in
four independent places — the `raisin:InboxTask` NodeType's `enum`
constraint, the `TaskType` enum plus an exhaustive `match` in the runtime,
the hardcoded allow-list in the `raisin.tasks.create` function binding, and
the CLI's flow validator. An application could not introduce a task type of
its own without an engine release, even though the engine has no semantic
stake in the vocabulary beyond the four it renders specially.

**Fixed:** the *set* is now open, the *shape* is still validated. `TaskType`
gained a `Custom(String)` variant with a slug rule
(`[a-z][a-z0-9_-]{0,63}`, `is_valid_task_type_slug` in
`types/flow_definition/config_types.rs`) and serializes as its bare slug;
the NodeType dropped its `enum` (v5, which drives the startup resync); the
function binding validates the slug's shape rather than its membership; the
CLI reports `INVALID_TASK_TYPE` on a malformed slug instead of
`UNKNOWN_TASK_TYPE` on an unfamiliar one. Client packages follow: the
designer's task-type control is a free-text field with the canonical four as
completions, the admin console's task badge falls back to a neutral chip
showing the type's own name rather than mislabelling it "Action", and its
flow snapshot round-trip preserves any well-formed slug instead of dropping
it.

### 1.3 No dynamic fan-out, so no per-item human-task join

**Was:** the `parallel` container read a **static** `branches` array from
its own properties, with no template resolution and an inline
`flow_definition` required per branch. A flow could therefore not fan out
one branch per runtime item, which made "one task per row, then join" —
the shape any per-item review or approval needs — inexpressible. The
workaround was to compute a roll-up from child state outside the flow, so
the flow never actually owned the join. The `loop` container could iterate a
runtime collection but only sequentially, and `Parallel`'s join could not be
used as a race either.

**Fixed:** the container now also accepts `for_each` (a collection
expression, resolved through the same shared resolver the loop step uses —
`handlers/collection.rs`) plus a `branch` template instantiated once per
item, with `item` and `index` bound. A branch may reference a **deployed**
flow by `flow_path` or carry an inline `flow_definition`. Fan-out width is
bounded by `max_branches` (default 500) and truncation is logged, so runtime
data cannot turn one step into unbounded child flows. The join was already
correct for this — it waits for every child to reach a terminal state, so a
branch parked on a human task is joined properly — but the merge output now
keys results by `branch_id` and emits an ordered `branches` array, because
positional keys alone lose which item produced which result. `branch_id` is
threaded through the `flow_execution` job payload for path-referenced
children. Reachable from the designer format too, via a container-level
`fan_out` config. See `docs/workflows.md` §3.3.

### 1.4 Async HTTP function invoke ran without the caller's identity

**Was:** `InvokeFunctionRequest.sync` defaults to false, so a plain HTTP
POST ran the function as a background job — and the job handler hardcoded
`None` for auth, dropping the caller's identity and failing any RLS-gated
operation with "No auth context set". The synchronous path was always
correct, which made this look like an auth bug rather than a sync/async
asymmetry.

**Fixed:** the invoke route serializes the caller's `AuthContext` into the
job metadata and the handler deserializes it
(`crates/raisin-rocksdb/src/jobs/handlers/function_execution.rs`). Jobs with
no such metadata key — trigger-invoked functions — still fall back to system
context, so trigger behaviour is unchanged.

### 1.5 Three step types forced the whole flow out of the designer format

**Was:** `wait`, `sub_flow`, and free-standing `decision` steps existed only
in the runtime format. Since the two formats cannot be mixed in one
definition, needing *any* of them — a cool-off period, a call to another
flow, a two-way branch outside an `or` container — meant hand-writing the
ENTIRE flow in the low-level format, giving up the visual designer and the
designer-level validation the CLI performs. Free-standing decisions were the
worst case: a designer step carrying a `condition` *did* lower to a
`Decision` node, but with neither `yes_branch` nor `no_branch` set, so the
handler rejected it at run time for a missing property. Representable, and
broken.

**Fixed:** `DesignerStepType` gained `Wait`, `SubFlow`, and `Decision`, with
the properties each needs (`wait_type`/`duration`/`until`/`event_type`/
`cron`; `flow_ref`/`input_mapping`/`async`; `yes_branch`/`no_branch`) and
lowering for all three. A decision names only the arm that diverges — the
unnamed one defaults to the sequential successor (`fill_decision_branches`),
which is the spelling designer format's array-order semantics call for. The
CLI validator, its `explain` plan output (including decision reachability
through *both* arms), and the client packages follow. See
`docs/workflows.md` §2.6.

### 1.6 A timed-out child never told its parent (join hung forever)

**Found by e2e testing the fan-out**, not by review. `fail_timed_out`
(`crates/raisin-flow-runtime/src/runtime/resume/mod.rs`) transitioned a
timed-out instance to `Failed`, saved it, and returned — but never called
`notify_parent_flow`, which every OTHER terminal transition does. So the
moment one child's wait expired, a parent parked on a join waited **forever**:
the child was `Failed`, the parent stayed `Waiting`, and no error surfaced
anywhere. It affected `sub_flow` too, but a fan-out with per-item deadlines
gets one chance to hit it per item, which is what made it worth finding.

**Fixed:** `fail_timed_out` now notifies the parent with the same
`"failed"` payload shape as the ordinary failure path. Regression test:
`e2e_flows::timed_out_child_notifies_parent_and_releases_the_join`.

### 1.7 A handler error left the instance stuck in `Running`

Also found by e2e. The execution loop treated a step handler returning
`Err(e)` differently from one returning `StepResult::Error { error }`: the
former `return Err(e)`'d straight out of `execute_flow` **without**
transitioning the instance, so it sat in `Running` forever — not failed, not
waiting, no error recorded, no retry, no error edge, no compensation, and no
parent notification. The two describe the same event and now take the same
path (`handle_error_result`). `FlowError::VersionConflict` is deliberately
still returned raw: it is infrastructural, and the job system must see it to
redeliver.

`parallel`'s `all_success` / `first_success` merges report failure exactly
this way, so a fan-out with either strategy and one failing branch produced a
zombie flow. `merge_all` never errors, which is why the pre-existing suite —
whose only parallel tests used `merge_all` — never caught it. Regression
test: `e2e_flows::fan_out_all_success_fails_the_join_when_a_branch_fails`.

**Related default changed:** a `parallel` container now defaults to
`max_retries: 0` (`runtime/executor/helpers.rs::get_max_retries`). Retrying a
fork/join re-forks every branch, and the branches already ran — for a fan-out
of human tasks that means a second task for every assignee. A container whose
branches are genuinely idempotent can opt back in with an explicit
`max_retries`.

### 1.8 Concurrent writers silently lost each other's updates

**The root cause behind a hung fan-out join, found by live testing.**
`save_instance_with_version`
(`crates/raisin-rocksdb/src/jobs/handlers/flow_callbacks/trait_impl.rs`) is a
read, a check, and a separate write — not a compare-and-swap. Two resumes of
one instance both loaded version N, both passed the check, and both wrote
N+1; the second carried a snapshot taken before the first, so the first
writer's accumulated state vanished. A parallel join waiting on that state
waited forever.

Observed live with two branches whose deadlines expired 3ms apart: both
children `Failed`, both notified the parent, and the parent recorded ONE
result and stayed `Waiting` permanently. The inline `VersionConflict` retry
that exists for exactly this (20 × 25ms in
`jobs/handlers/flow_instance_execution.rs`) never fired, because no conflict
was ever detected — one unfinished primitive silently disabled a correct
mechanism sitting on top of it.

**Fixed** by serializing execution per instance rather than by making every
flow write conditional: `jobs/flow_instance_lock.rs` takes a per-instance
lock before any write, in-process via `KeyedMutex` (queues, and evicts
entries on release) plus an optional `raisin-locks` lease for cluster-wide
scope. That is the shape mature engines use — one outstanding task per
workflow execution, or single-threaded command processing per instance
partition — and it makes the optimistic path a genuine second line of
defence instead of decorative. Verified live: 5 branches sharing one
deadline now all land and the parent completes, where 2 branches used to
hang.

**Also fixed alongside it:** cancelling a child never notified its parent
(`service.rs::cancel_instance`), the same hang as §1.6 by a different route.

---

## 2. Open engine items

### 2.1 [P2] Flow-instance serialization guards the job handler, not the aggregate

**Symptom:** the per-instance lock (§1.8) is taken by the job handler, so a
writer that does not go through it is unprotected. `service.rs`'s
`cancel_instance` still writes the instance node directly
(`storage.nodes().update`, `validate_schema: false`) with no lock and no
version check, so it can clobber a live execution's snapshot — the very bug
the lock exists to prevent. It now at least notifies the parent (§1.8).

**Verdict: fix in the engine.** Two candidate depths, ascending:

1. Route cancel through the runtime (enqueue a cancel job, or take the same
   instance lock) so every mutation shares one path.
2. Hoist the lock into `raisin-flow-runtime` as a `with_instance_lock(id, …)`
   wrapper that `execute_flow` / `resume_flow` / `check_flow_timeout` / cancel
   all pass through, with the lock manager supplied via `FlowCallbacks`. That
   puts the invariant next to the aggregate that owns it instead of in
   `raisin-rocksdb/jobs`.

A genuine storage-level CAS is the third option and deliberately deferred: the
instance is persisted through opaque node callbacks that carry no version
(`flow_callbacks/types.rs`), and the node layer has no conditional write at
all (`UpdateNodeOptions` has no revision predicate, and there is no
`TransactionDB` in `raisin-rocksdb`). It would mean adding `expected_revision`
to `UpdateNodeOptions`, enforcing it in the rocksdb update path, and widening
two callback aliases — worth doing, but multi-crate, and the lock is needed
regardless (queueing beats conflict-retry latency).

### 2.2 [P3] Terminal transitions notify the parent from four separate sites

**Symptom:** "set terminal status → persist → emit event → notify parent" is
hand-rolled at four sites (complete, fail, timeout, cancel). Two of them were
missing the notification and each was fixed separately (§1.6, §1.8) — a fix
applied twice at two sites is the signal the logic belongs one level down.

**Verdict: worth funnelling.** One `terminate_instance(instance, status,
output, error, callbacks)` that stamps status/`completed_at`, persists, emits
the matching `flow_*` event, and notifies the parent; every site calls it.
Deferred only because the correctness holes are closed and the refactor
touches the rollback path too (`runtime/compensation.rs` sets `RolledBack`
and relies on `fail_flow` notifying afterwards).

### 2.3 [P3] Fan-out re-resolves the whole context per templated field

**Symptom:** `plan_fan_out_branches` clones the `FlowContext` per item, and
every `DataMapper::map` call ends in `context.to_json()` — a deep copy of
input + all step outputs + variables — then re-parses the expression. At the
documented ceiling (500 branches, ~3 templated fields) that is ~500 context
clones plus ~3000 full traversals for a step whose context never changes
except for two keys.

**Verdict: fix in the engine, but it is a `DataMapper` change, not a fan-out
one** — the same cost applies to any step with several templated fields;
fan-out just multiplies it by N. The fix is an entry point that takes a
pre-built `EvalContext` (and ideally a pre-parsed expression) so
`to_json`/`from_json`/parse happen once per step instead of once per
expression per item, with `item`/`index` overwritten in the prepared map.
Related: a `flow_path` fan-out reloads the SAME deployed flow definition once
per branch, sequentially, inside `queue_job`; memoize it per path and queue
with bounded concurrency.

### 2.4 [P3] Fan-out hardcodes the `item` / `index` binding

The loop container supports configurable `item_var` / `index_var`; the
fan-out binds `item` and `index` unconditionally. Two conventions for one
concept, and the fan-out's names are now in published docs. A shared
`bind_item(context, item, index, item_var, index_var)` next to
`resolve_collection` in `handlers/collection.rs` would let both agree — and
would be the natural home for a shared default iteration cap (`parallel` has
`max_branches` = 500; the loop's `max_iterations` still has no default).


### 2.5 [P2] No reminder tiers on a human task

**Symptom:** every escalation tier has to be hand-wired as its own
`human_task` + `timeout_edge` + notify function. A three-tier escalation is
a few hundred lines of flow YAML that says the same thing three times.

**Root cause:** a wait carries exactly ONE deadline. `due_in_seconds`
becomes the wait's `timeout_ms`, and expiry is a single terminal event
(route to `timeout_edge`, or fail). There is no notion of "fire a
notification at T, then again at 2T, and only escalate at 3T".

**Verdict: worth fixing in the engine, not yet done.** A
`reminders: [{ after, notify }]` list on a human task is a genuine
engine-level capability — it needs multiple timers per wait, which the wait
sweeper cannot express today and an application cannot bolt on without
polling. Sketch: keep the terminal deadline as-is, add non-terminal reminder
checkpoints to `WaitInfo` that the sweeper fires and marks consumed
(idempotently, since the sweeper may re-deliver).

**Until then:** the escalation ladder is authored explicitly —
`human_task` → `timeout_edge` → notify function → second `human_task` with
its own shorter deadline → final `timeout_edge` to a function that
synthesizes a safe default. Have every tier converge on the SAME resolver
step so audit and side-effect behaviour cannot drift between tiers.

### 2.6 [P2] SQL against `raisin:access_control` can return zero rows silently

**Symptom:** `SELECT * FROM "raisin:access_control" WHERE node_type =
'raisin:Role'` — and even an unfiltered `SELECT * … LIMIT 20` — returns
`{columns: [], rows: [], row_count: 0}` with no error, for a session whose
token reads the very same nodes fine by path
(`GET /api/repository/{repo}/{branch}/head/raisin:access_control/roles/{name}`).
The table name resolves, the query plans, it just never yields rows.

**Root cause: not established.** It is session- or query-shape-specific
rather than universally broken — scripted SQL against this workspace is
known to work in other setups. The leading hypothesis is that row-level
enforcement on the scan path treats this workspace as default-deny while the
by-path read does not, i.e. an inconsistency between the two enforcement
surfaces rather than a scan bug.

**Verdict: fix in the engine once reproduced.** Silent zero rows is the
worst possible failure mode for an access-control audit: it reads as "no
roles exist". Whatever the resolution, the scan and by-path surfaces must
agree, and a denied scan should be an error, not an empty result.

**Repro recipe for whoever picks this up:** with a non-superadmin token that
can read the workspace by path, run the unfiltered `SELECT` and the
equivalent by-path `GET` side by side; then re-run the `SELECT` with RLS
disabled for the session to confirm or kill the enforcement hypothesis.

**Until then:** enumerate this workspace over REST by path. Note the
limitation: that only finds roles, groups, and users whose paths you can
already guess, so it does not scale to *discovering* unknown ones.

### 2.7 [P2] Access-control content cannot be updated by deploy or sync

**Symptom:** `deploy --install` **creates** new nodes in
`raisin:access_control` cleanly, but does not update existing ones, and a
force push of the workspace fails outright. Every subsequent change to a
role's permission list is therefore a manual admin-console edit, and the
declarative package copy silently drifts from what is live. A workspace
introduced with a package needs a hand-applied grant on day one before an
application-level (non-function) caller can write to it at all.

**Root cause:** the workspace is deliberately special — it is the store
that backs authorization itself, so the ordinary content write paths are not
open to it.

**Verdict: worth fixing in the engine, carefully.** "Declarative
everywhere except the one workspace that decides who can do anything" is a
real gap, and drift between the declared and live permission sets is a
security problem, not a convenience one. But a package that can silently
rewrite roles is an obvious privilege-escalation path, so this needs a
designed answer — an explicit, separately-authorized reconcile step that
diffs declared against live and requires an operator to approve the diff —
rather than simply opening the workspace to `deploy --install`.

**Until then:** treat the package copy as the source of truth, apply changes
by hand through the admin console, and verify with a by-path REST read of
the role node (see 2.6 — do not trust a SQL enumeration here).

### 2.8 [P3] Reference and section field type constraints are advisory only

**Symptom:** `ReferenceField.allowed_entry_types` cannot express "any type
in workspace X", so a picker that should be governed by policy has to
enumerate concrete types instead. Separately,
`SectionField.allowed_element_types` acts as a hard ceiling that policies
can only narrow, so widening what an archetype accepts means editing that
archetype's YAML.

**Root cause:** `allowed_entry_types` is a **client-side hint** — it is
declared on the field config
(`crates/raisin-models/src/nodes/types/{element,block}/fields/reference_field_config.rs`)
but nothing in validation enforces it, so the server already accepts any
reference. `allowed_element_types` *is* enforced
(`crates/raisin-core/src/services/node_validation/element_validation.rs`),
by design: it is the schema's own statement of what an archetype is.

**Verdict: mostly documentation, not code.** For references, "any type in
the workspace" is expressed by simply omitting `allowed_entry_types` — the
constraint that seemed missing is the absence of a constraint, and clients
should treat an omitted list as "unconstrained" rather than "none". Clients
that want a wildcard spelling may accept `["*"]` as a synonym for omitted.
For sections the ceiling is intentional and stays: an archetype's element
whitelist is content, versioned and reviewed with the package, and policies
narrowing rather than widening it is the property that makes the ceiling
worth anything.

---

## 3. Not engine concerns

Gaps that surfaced as engine requests but are better solved with primitives
already shipped. Listed with the pattern that works, so they do not come
back as feature requests.

| Gap | Solve it with |
|---|---|
| **No manual "run a flow over a selection" surface.** Starting a flow for each of N selected records has to be coded per use case. | `flows.run(path, input)` per record from the application, or one flow whose input is the selection and whose first container is a `for_each` fan-out (§1.3). A generic "which flows may run on this node type" binding is an application-level config node, not an engine primitive — the engine should not own the app's action registry. |
| **Complementary-condition sibling containers as if/else is unproven.** | Don't emulate if/else — the `decision` step *is* if/else: one REL `condition`, explicit `yes_branch` / `no_branch`, exactly one arm taken. Two sibling containers with opposite conditions is a fragile spelling with no single point of decision. Now authorable in the designer format (§1.5), so this no longer forces the runtime format; use an `or` container for more than two cases. `docs/workflows.md` §2.6. |
| **Roll-up status must be computed from children instead of being authoritative from a flow.** | Now expressible in the flow: a `for_each` fan-out with `merge_strategy: all_success` makes the join itself authoritative (§1.3). Derive-from-children remains the right choice only when the children outlive the flow instance. |
| **No search box or facet filter in a content browser.** | SQL over the workspace, with `->>'key'::String` property predicates, `COUNT(*)`, `ORDER BY`, `LIMIT`, and keyset pagination on `__order` / `__tree_order`; full-text via the fulltext index. The engine has the query surface; the browser UI is the application's. |
| **A reference listing has no LIMIT or server-side filter.** | Same: push the filter, `ORDER BY`, and `LIMIT` into the SQL instead of listing everything and filtering client-side. `REFERENCES('ws:/path')` composes with `DESCENDANT_OF` / `CHILD_OF` / `node_type` / property predicates, `ORDER BY`, `LIMIT`, `COUNT(*)`, and bound parameters. |
| **Reference expansion is single-pass with no cycle guard.** | Client-side resolution concern — the engine returns references, it does not decide how deeply a client expands them. Bound the depth and keep a visited set in the resolver. |
| **Nothing auto-places a created node at the path implied by a field value.** | A node-event trigger on create plus a function that moves the node, if the placement must be enforced rather than merely defaulted. Making the engine derive paths from field values would put application routing rules into the storage layer — the wrong place for them. |

---

## 4. Notes and gotchas

Behaviour that is correct but surprising. Documented rather than "fixed",
and worth reading before filing a bug.

- **`waiting` does not mean "parked for a human".** The runtime emits a
  transient `waiting` status *between* steps while a queued execution is in
  flight. Discriminate on the **wait reason** in `WaitInfo` (`human_task`,
  `parallel_branches`, `sub_flow`), not on the bare status.
- **Completing a task the instant it is listed can race the park.** The
  inbox task node becomes listable slightly before the owning instance's
  status record settles to `Waiting`; completing in that window returns
  `Invalid state transition from pending to resumed`. Retry, or poll the
  instance to `waiting` first. Both are documented in `docs/workflows.md`
  §6.2.
