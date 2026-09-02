# Fair, tiered job scheduling with a global safety ceiling

## Why

2026-09-02: an extraction-generation bump re-opened every asset on the host while
the embedding upstream (Infomaniak, via marvel) was returning 503. Measured over
one 5-minute window:

| | |
|---|---|
| distinct nodes | 257 (spanning every tenant on the box) |
| embedding attempts | 2,512 (~10 per node; one node 44×) |
| successful embeddings | 3 |
| upstream 503s | 476 |
| host CPU | **96%** on 16 cores |

The concurrency cap WORKED — `CategoryPool` holds one `Semaphore(20)` for the
Background category (`config/mod.rs:241-245`, production preset) and never
exceeded it. The damage came from three things the cap cannot express.

## The three gaps

### 1. No per-tenant fairness — the scheduler has no idea tenants exist

`pool.rs` contains no occurrence of the word `tenant`. One process-wide
semaphore, fed by one FIFO channel per category. So a tenant that enqueues 3,000
jobs puts them all ahead of every other tenant's next job. With 50 tenants
remapping, the 50th waits for the other 49 to drain.

This is structural: a global semaphore can cap TOTAL concurrency but cannot
express "no tenant may crowd out another".

### 2. No circuit breaker — every job rediscovers the outage alone

476 identical 503s in five minutes, each one preceded by the expensive work
(extract, chunk, tokenize) and only THEN discovering the upstream was down. The
cheap failure happens last. Nothing shares the knowledge "this provider is
returning 503" between jobs.

### 3. Retries redo work that is already stored and deterministic

Extraction output is stored on the node and is a pure function of the bytes. A
retry re-runs it anyway. Chunking likewise.

### Resolved: there is NO retry bug, and no runaway cascade

The "10-44 attempts per node" that looked impossible was a arithmetic error on my
part. `"About to generate embedding"` is logged once per SPEC per attempt
(`embedding/handler.rs:310`, inside `embed_one_spec`), not once per job. A node
with two specs (`default` + `doc`) running its full budget of 1 initial + 3
retries produces **up to 8 lines**. 2,512 lines over 257 nodes is 9.8 each —
i.e. roughly ONE job per node, correctly capped. The 44-line outlier is ~5 jobs,
i.e. five node revisions.

So `max_retries` was respected throughout (`status.rs:279-360`,
`worker.rs:495-528`), the embedding handler never writes to the node (no
write→event→enqueue loop), and asset processing's fingerprint gate held. The
volume was simply 257 assets re-opened at once, each doing four doomed attempts
against a dead upstream, each attempt re-chunking first.

**This makes the incident simpler and the remedy narrower**: nothing needs
un-looping. What is missing is a breaker, fairness, and not redoing work.

### Secondary finding: a documented tradeoff that inverts during an outage

`job_helpers.rs:64-67` appends `context.revision` to the dedup key for
`EmbeddingGenerate` only, so every node revision mints a fresh job with a fresh
retry budget. The comment at `:50-58` justifies this explicitly: a run whose
inputs have not moved "carries the previous revision's vectors forward instead of
calling the provider... the extra runs a burst now produces cost a row write each
and no provider call."

That justification is sound **only while a prior good vector exists to carry
forward**. During an outage there is nothing to carry, so every "cheap" extra run
becomes a full 3-retry cycle against the failing upstream. The assumption is
correct in the healthy case and silently inverts in exactly the case that hurts.

The breaker (B) neutralises this without touching the dedup key: with the breaker
open, a fresh job parks instead of attempting. Worth a comment amendment at that
site so the next reader knows the tradeoff is outage-sensitive.

## Design

### A. Fair-share admission with tier weights

Replace the per-category FIFO with **deficit round-robin over per-tenant queues**.

- Each tenant gets a queue; the scheduler serves them in rounds, granting each a
  credit proportional to its **tier weight**.
- An idle tenant costs nothing: DRR skips empty queues, so ONE active tenant may
  use the whole global budget. This is the "fair usage, not a hard cap" the
  product wants — a hard per-tenant cap would leave the machine idle while work
  waits.
- Under contention, share is proportional to weight. A higher tier gets served
  more often, never exclusively: every non-empty queue is visited each round, so
  a free-tier tenant cannot be starved by a paying one.
- The global `Semaphore` STAYS, unchanged. It is the machine-safety ceiling and
  the fair scheduler sits in front of it, deciding WHOSE job takes the next
  permit — not how many run.

#### What this guarantees, concretely

Two properties, and they are the whole point:

**Progress.** A tenant with 100,000 queued jobs cannot delay another tenant's
single job by more than ONE scheduling round. The scheduler visits every
non-empty queue each round, so the small tenant's job waits behind at most one
job per competing tenant — never behind the backlog. Today it waits behind all
100,000, because the queue is a single FIFO.

**Priority without starvation.** Tier sets the credit granted per round, so a
paid tenant is served more often than a free one. It is a RATIO, not a
precedence: with weights 4 (paid) and 1 (free), a round serves roughly four paid
jobs per free job. The free tenant still advances on every round.

Worked example, one active free tenant with 100 queued jobs and one paid tenant
arriving with 3:

| round | free (w=1) | paid (w=4) |
|---|---|---|
| 1 | 1 job | 3 jobs (its whole backlog) |
| 2+ | 1 job/round | idle — skipped, costs nothing |

The paid tenant clears immediately because its weight buys more credit per round
AND the free tenant's backlog never blocks the queue head. Once paid is empty,
free gets the entire machine again — DRR skips empty queues, so weights only
matter under contention.

Strict priority (drain all paid before any free) is DELIBERATELY NOT the design:
it starves the lowest tier indefinitely whenever a paid tenant has a backlog,
which is exactly the failure the customer of a free tier would experience as
"the product is broken".

Queues must be bounded (the existing 20k capacity valve applies per tenant, not
globally, or one tenant can still exhaust the shared bound).

### B. Circuit breaker, keyed by upstream and shared across tenants

- Key: the provider/upstream identity (e.g. `marvel:infomaniakprod`), NOT the
  tenant — an upstream outage is not a tenant's fault and every tenant learns it
  at once.
- Closed → Open after N consecutive 5xx/transport failures in a window.
- While Open, a job that needs that upstream is **parked before the expensive
  work**, not attempted. It does not consume a handler permit and does not
  re-extract.
- Half-open probes a single request; success closes it.
- A breaker that is open must be VISIBLE (see D) or it becomes a silent stall.

### C. Resumable job steps

A job that failed at the embedding step must resume there, reusing the stored
extraction and (where the spec hash is unchanged) the stored chunking. The spec
hash already guards the provider call; the CPU work in front of it is what needs
the same guard.

### D. Surfacing in the admin console

Nothing today shows queue depth, per-tenant share, or upstream health — the
outage was discovered from a CPU graph. Add:

- **Queues**: per category, per tenant — depth, in-flight, tier/weight, oldest
  job age. Answers "who is using the machine right now".
- **Breakers**: per upstream — state, consecutive failures, last error, next
  probe. Answers "why is nothing progressing".
- **Retry/parked**: counts and reasons.

Read-only first; operator actions (drain a tenant, force-close a breaker) after.

## Multi-node caveat

Job dedup and these pools are **per-process**. On an N-node cluster each node
runs its own scheduler and its own breaker. Fairness is therefore per-node, which
is correct for CPU protection (the resource being protected is that box) but
means an upstream outage is discovered N times. Cluster-wide coordination is out
of scope here and should stay so until there is a reason.

## Sequencing

1. **Circuit breaker** — smallest change, largest immediate protection, and it
   alone would have prevented today.
2. **Admin surfacing** for queues + breakers — so the next incident is legible.
3. **Fair-share scheduler** — the biggest change, and the one that needs the most
   care; it sits in the hot path of every job in the system.
4. **Resumable steps** — optimisation once correctness is in place.
5. **Amend the dedup-by-revision comment** to record that its cost argument
   depends on the embedder being healthy.

## Verification

- A unit test that one tenant enqueueing 10k jobs does not delay another
  tenant's single job beyond one scheduling round.
- A test that an open breaker parks jobs WITHOUT running the handler body.
- A load test: 50 simulated tenants each remapping, asserting bounded host CPU
  and that every tenant makes progress.
- Regression: with the breaker open, N revisions of a node produce N parked jobs
  and ZERO provider calls — the case that produced today's volume.

---

## Reading the activity node on a cluster

`raisin:JobActivity` at `job_activity:/activity` is written by whichever process
observed the work. Every write carries `origin` and `updated_at`, so a reader can
always state exactly what it holds — "as of 14:03:07, process `node-2` saw this
repo at active=3, not degraded". That is a correctly-labelled SAMPLE, never a
claim about the cluster.

Under last-writer-wins the two halves of the node behave differently, which is
why a blanket "do not trust this on a cluster" would be wrong — it is stronger
than the truth and would get the whole node ignored.

**Rule 1 — `degraded` is cluster-safe. Read it as-is.** Derived indexing runs on
every node (nothing indexed replicates), so an embedding job for a repo runs on
all of them. During an outage they all park against the same upstream and all
write `true`: they agree, and whichever wins the race is right. On recovery each
clears as its own job succeeds, so the bit flaps briefly before settling `false`
— self-correcting, and it errs toward showing the banner slightly too long
rather than clearing it early.

**Rule 2 — the counts are a LOWER BOUND, not a total.** `active`, `active_paths`
and `tenant_pending` are one process's slice. A reader sees node A's 3, then node
B's 0, then A's 2, with no way to distinguish "the work finished" from "a
different node answered". Never conclude "idle" from `active == 0` alone on a
clustered deployment; read it as "at least N, on the process named in `origin`".

The consequence that would actually mislead: **`active: 0` from one node does not
mean the cluster is idle.** A progress indicator that treats zero as "done"
disappears while another node is still working. "The count went to zero" is the
obvious trigger for dismissing such an indicator and is precisely the one that
breaks here.

On a single node — what runs today — both readings are exact. This is a caveat to
write down before anyone runs it clustered, not a bug to fix now.

Write volume: the 2s floor is per-process per-repo, so three nodes cost at most
~1.5 writes/sec/repo at worst and zero when idle — the idle rule holds
independently on each node, so a quiet cluster is as quiet as a quiet single one.
