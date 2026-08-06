# Virtual Nodes: External Systems as Content

*Experimental / preview.*

A virtual node is an ordinary node whose content originates outside RaisinDB. A mount points at a folder in a Drive, a mailbox, a calendar; the sync engine materializes what it finds there as real, committed nodes. They are not a read-only overlay — they carry revisions, fire triggers, answer SQL, get indexed and take part in workflows exactly as authored content does. That is the whole point: everything the database can already do to content applies to a mailbox the moment it is mounted.

The pieces:

| Node | Role |
|---|---|
| `raisin:Integration` | A connector: which provider, which OAuth app, which adapter function. |
| `raisin:VirtualMount` | One binding of a remote container to a workspace path, with its sync and write configuration. |
| adapter function | JavaScript. Talks to the provider's API and returns normalized items. |
| mapping function | JavaScript. Translates one provider item into a node — and, for writes, back again. |

The engine is domain-blind. It knows how to page, reconcile, retry and commit; it does not know what a calendar is or that a mail body is immutable. Everything provider-specific lives in the two JavaScript functions, which is why adding a connector is a package rather than a patch.

## Adapters never write nodes

An adapter is a function the engine calls: it takes a request, hits the provider, returns a result. Every local write is performed by the engine.

This is worth stating because the alternative is tempting and loses four things at once. The engine's per-mount lease serializes writes; the stamp-back is what prevents an echo loop; the blast-radius rails bound a delete; and adapters run privileged with a system auth context, so an adapter that could write nodes would have unrestricted workspace access from a sandbox. The division is: **the adapter decides what the remote becomes and performs the remote call; the engine decides what the node becomes and performs the local write.**

## The write path

Writing back is opt-in per mount and off by default. The reason it is off rather than merely unconfigured is that a wrong write configuration reaches somebody else's mailbox or calendar — it is not a setting to inherit.

The generalization that makes one mechanism serve every provider is that **the write mode belongs to the mount, not to the adapter**. The same Microsoft 365 connector serves a `state_only` inbox and a `submit` outbox.

| Mode | Example | A local change means |
|---|---|---|
| `state_only` | a mail message | Content is immutable; a declared allow-list of properties pushes (the read flag, flags, the folder). |
| `mirror` | a calendar event, a Drive file | The node **is** the remote object — create, update and delete propagate. |
| `submit` | send a mail, RSVP | The node is a **command** with a status lifecycle, not a mirror of anything. |

`submit` is what makes mail coherent. An email is immutable, so its write path is a *sending* path, and the natural home for that is a separate mount whose members are intents. `reply` and `forward` then need no special casing: the outbox node carries an action and the id of the message it answers.

### Change detection reads revision metadata

The obvious approach does not work, and the reasons are worth recording because each looks correct until tried.

Filtering on `updated_by != "virtual-mount-sync"` matches every node the sync itself wrote — the transaction's auth context wins over the raw actor, so the stamp is `system`. Comparing `updated_at` against the stored sync timestamp is always true, because one is captured when an item is staged and the other when the batch commits, minutes apart on a large page. And deletes are structurally invisible to any sweep over live nodes: the node is gone, and there is no per-mount manifest of the external ids it owned.

So detection reads `RevisionMeta` — one HLC-ordered scan that is durable, ordered, delete-visible and correctly attributed — from a watermark stored on the mount. A capture hook on the event bus provides the low-latency path, and it is allowed to be noisy.

### Idempotence, not precision

The layer that carries the design is neither of those. Before any push, the engine re-reads the node and asks whether the change is already at the provider; if it is, no adapter call happens. That is what lets detection be sloppy — the same architecture the read path already uses, where an etag skip-write lets the delta feed be sloppy.

The comparison is against `__pushed_state`, a stamped record of the watched properties as they were sent, **not** against the etag. The etag would require the token returned by a write to be byte-identical to the one the next delta reports for that item, which no provider guarantees: Microsoft's `isRead` does bump the message's change key, while IMAP's `\Seen` does not move `MODSEQ` usefully. `__pushed_state` converges regardless of what the etag does; the etag skip is the optimization that keeps a converged item from being re-mapped.

### Provenance for nodes the mount does not own

Every other write path starts from a node carrying `__mount_id` and `__external_id`. A node the user just authored under the mount path carries neither — that absence is exactly what makes it a candidate to create remotely, or to issue as a command.

The answer is an explicit node-type allow-list on the mount (`create_node_types`, `command_node_types`), **empty by default with no fallback**. Guessing wrong uploads private content to a third party, and no later configuration change undoes that. It is the same reasoning that gives `mutable_fields` no "all fields" value.

### Deleting is a policy

"Delete" is ambiguous at every provider that has a recycle bin, so it is resolved as engine default < adapter default < mount override, and the ambiguous reading is never the destructive one.

- `detach` — nothing is pushed. Honestly documented: the node is removed locally and the next full reconcile **re-imports it**, because there is no per-mount suppression set.
- `trash` — a recoverable soft delete. Requires the adapter to declare `supports_trash`; a mount asking for it against an adapter that has no trash is **refused**, never quietly promoted to a permanent delete.
- `purge` — irreversible, and never a default at any layer. An operator types it.

The rails around it are proportional: a drain is blocked when pending deletes exceed `max(floor, ratio × mount size)`. An absolute cap alone is wrong in both directions — far too low for a ten-node mount, far too high for a 200,000-message mailbox. A delete whose originating transaction touched many nodes is flagged as bulk regardless of how few deletes reach any one drain, which is how a mis-scoped `DELETE ... WHERE path LIKE '/mail/%'` is caught. A tripped rail parks the deletes and blocks outbound writes only; reading carries on.

### Commands are issued at most once

`submit` deliberately bypasses the retry loop that serves the other modes, because a retry sends someone a second email.

A durable `queued → sending` transition is committed before any network call, stamped with an attempt id. On success the node moves to `sent`. A rate limit returns it to `queued`. A definitive rejection fails it. **Everything else parks at `unknown` and is never retried automatically** — including a crash between the transition and the response, which the next drain moves to `unknown` rather than back to `queued`. The durable pre-write exists to convert an unbounded ambiguity into a bounded one: the question becomes "did this one attempt land?" rather than "how many were there?".

### Conflict

A conflict is the provider saying the object changed since this mount last read it. It is not a fault, and what it means is a decision the engine cannot make:

- `remote_wins` (default) — abandon the push and let the next sync overwrite the local value. A lost edit the user can redo.
- `local_wins` — re-send with no concurrency base, overwriting the other writer. Not recoverable.
- `error` — park the edit and report it.
- `resolver_function` — hand the decision to a function shipped by the package, which answers `local_wins`, `remote_wins`, `merged` or `park`. Anything unclear — a throw, an unrecognized answer — parks rather than guesses.

## Attachments, and content on demand

A sync writes attachment **metadata** only: a `raisin:Asset` child per attachment with its name, mime type and size. Downloading every attachment of every message during a sync would multiply a mailbox import by whole documents, and most attachments are never opened. `file == null` on a mount-owned asset means precisely "not fetched yet"; the bytes are fetched when something asks for them and stored like any other asset's.

Attachments are children rather than an array property because the Drive adapter already maps provider blobs to `raisin:Asset` — an array would build a second, parallel blob path. As children they are queryable, can be full-text indexed, and inline images fall out naturally through a `content_id`.

## Calendars converge on RFC 5545

The two calendar providers were exact opposites: one returns series masters carrying a proprietary recurrence object, the other expands every occurrence and never shows a master at all. `raisin:Event` is neither. It stores a series master with recurrence as RFC 5545 content lines, exceptions as sibling nodes, and an explicit IANA timezone — because "every Tuesday at 09:00 Europe/Zurich" is a different instant in winter and in summer.

RFC 5545 is canonical because Google, CalDAV and Apple use it natively; Microsoft is the outlier and converts. A window-bounded expander materializes concrete occurrences as a derived projection, so a date-range query stops missing every recurring event. The write path only ever touches masters and exceptions — an edit to a generated occurrence is refused with a pointer to its master, which is what real calendar systems do.

## Where configuration lives

Mounts and integrations live on the repository's **config branch**; the nodes they materialize land on the mount's **target branch**. The two are independent and must not be conflated — a sync reads its configuration from one and writes content to the other, and passing the target branch for both resolves no mount at all.

## Further reading

- [Virtual node adapter contract](https://raisindb.com/docs/reference/virtual-node-adapters) — the frozen, package-facing API: operations, capabilities, error codes, and both directions of the mapping function.
- [Virtual Nodes concepts](https://raisindb.com/docs/concepts/virtual-nodes) — what you can mount today, and the guides for each connector.
