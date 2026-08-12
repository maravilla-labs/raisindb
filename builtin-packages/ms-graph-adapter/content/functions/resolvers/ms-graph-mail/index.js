/**
 * Microsoft 365 mail CONFLICT RESOLVER.
 *
 * Point a mail mount at this with `resolver_function: /resolvers/ms-graph-mail`
 * and it decides, per message, what a refused push means. Without a resolver a
 * mount falls back to `conflict: "remote_wins"` — the local edit is thrown away
 * and the next sync overwrites it — which is safe but silent.
 *
 * The engine invokes it through the same plumbing as the mapper (no separate
 * executor, no I/O allowed, runs under the mount lease):
 *
 *   { operation: "resolve_conflict",
 *     local,        // the node as it stands now
 *     remote,       // ALWAYS null - see below
 *     conflict,     // the adapter's own refusal message, verbatim
 *     base_etag,    // the concurrency base the refused push carried
 *     field_diff,   // { field: { local, pushed } } for every pushable field
 *     mount }
 *
 * returning
 *
 *   { resolution: "local_wins" | "remote_wins" | "merged" | "park",
 *     fields?, reason? }
 *
 * `remote` is null and will stay null: the adapter contract has no single-item
 * `get`, so there is no remote object to hand over. A resolver reasons about
 * `field_diff` — what the user changed since the provider last agreed — and
 * about `conflict`, which is the provider's own account of the refusal. Anything
 * that needs the remote VALUE has to `park` and let a human look.
 *
 * ---
 *
 * ## The rule this implements, and why
 *
 * A Graph conflict on a `state_only` mail mount is a 412/409 on a PATCH of
 * `isRead`. What it means in practice is that the message changed in Outlook
 * between this mount's last read and this push — most often because someone
 * opened it, which is itself a change to `isRead`.
 *
 * So:
 *
 *   - the user MARKED IT UNREAD (local true, pushed false): that is a deliberate
 *     act with intent behind it — "come back to this" — and it survives. The
 *     remote change that raced it was almost certainly the passive one (Outlook
 *     marking it read on open). local_wins.
 *
 *   - the user MARKED IT READ (local false, pushed true): the remote is at least
 *     as far along as we are on the only axis that matters, and forcing it would
 *     overwrite a change we cannot see for no gain. remote_wins.
 *
 *   - anything else — a field this resolver was not written for, or a diff with
 *     no local change in it at all — PARKS. An unknown case must never fall
 *     through to a guess that reaches someone's mailbox.
 *
 * Note that `unread` alone is never a `merged` case: it is a boolean, so there
 * is no third value to merge to. `merged` is written below for `labels`
 * (Outlook categories) because that is where it is genuinely the right answer —
 * two people adding different categories should end up with both, not with one
 * of them silently dropped.
 *
 * `labels` is NOT live yet: this adapter's `capabilities` declares
 * `mutable_fields: ["unread", "is_read"]` only, so the engine drops a merged `labels` as
 * outside the allow-list (with a warning) and the edit parks. The branch is here
 * so the rule ships with the resolver rather than after it — the day the adapter
 * PATCHes `categories` and adds it to `mutable_fields`, merging starts working
 * with no change here. Until then a `labels` conflict is parked, which is the
 * correct answer for a field this mount cannot push anyway.
 */

function handler(input) {
  if (!input || input.operation !== "resolve_conflict") {
    // Not addressed to us. Parking is the only safe answer: returning null (or
    // anything unrecognized) parks anyway, and saying why makes the
    // misconfiguration visible in writeback_last_error instead of looking like
    // a resolver that keeps declining.
    return {
      resolution: "park",
      reason: "this function only handles operation 'resolve_conflict'",
    };
  }

  var diff = input.field_diff || {};

  // Categories first: the one field here with a real merge.
  if (diff.labels) {
    var merged = unionLabels(diff.labels.local, diff.labels.pushed);
    if (merged) return { resolution: "merged", fields: { labels: merged } };
  }

  var unread = diff.unread;
  if (unread && typeof unread.local === "boolean") {
    // Marked unread locally, and that is not what the provider was last told.
    if (unread.local === true && unread.pushed !== true) {
      return { resolution: "local_wins" };
    }
    if (unread.local === false && unread.pushed !== false) {
      return { resolution: "remote_wins" };
    }
  }

  // `is_read` is the Graph-truth ALIAS of `unread` (the adapter's capabilities
  // declare both; the mapper folds them into one `isRead` on the wire). Same
  // two rules, inverted polarity: is_read=false is the deliberate "come back
  // to this" and survives; is_read=true yields to a remote that is at least as
  // far along. Checked AFTER `unread` so that when both spellings appear in
  // one diff the canonical column decides — mirroring the mapper's own
  // precedence, which is what keeps the resolver's verdict consistent with the
  // payload the refused push actually carried.
  var isRead = diff.is_read;
  if (isRead && typeof isRead.local === "boolean") {
    if (isRead.local === false && isRead.pushed !== false) {
      return { resolution: "local_wins" };
    }
    if (isRead.local === true && isRead.pushed !== true) {
      return { resolution: "remote_wins" };
    }
  }

  return {
    resolution: "park",
    reason:
      "no rule for this conflict (" +
      (input.conflict || "no message") +
      "); the edit is left pending",
  };
}

/**
 * Both sides' categories, de-duplicated, order-stable.
 *
 * Returns null when there is nothing to merge at all, so the caller falls
 * through to the `unread` rules rather than issuing an empty PATCH — which the
 * adapter refuses anyway, because it would bump the message's change key,
 * invalidate every stored etag and make the next delta re-deliver the message.
 */
function unionLabels(local, pushed) {
  if (!Array.isArray(local)) return null;
  var out = [];
  var seen = {};
  var sides = [local, Array.isArray(pushed) ? pushed : []];
  for (var s = 0; s < sides.length; s++) {
    for (var i = 0; i < sides[s].length; i++) {
      var label = sides[s][i];
      if (typeof label !== "string" || seen[label]) continue;
      seen[label] = true;
      out.push(label);
    }
  }
  return out.length ? out : null;
}
