/**
 * Google Calendar virtual-node adapter (EXPERIMENTAL / PREVIEW).
 *
 * Implements the frozen adapter contract (docs/reference/virtual-node-adapters.md)
 * over the Google Calendar v3 REST API using the synchronous `raisin.http.fetch`
 * binding. The sync engine invokes this function directly, decrypts the account
 * credential just before the call, and materializes returned items into nodes.
 *
 * Entrypoint: handler(input) — exactly one argument.
 *   input = { operation, params, credential, mount }
 *
 * Reads: can_read + supports_changes, plus push (supports_push) via
 * events.watch channels — notifications are pure invalidation signals.
 *
 * Writes: the full MIRROR set — create, update and delete — plus ONE command,
 * `submit`, which is an RSVP.
 *
 * An RSVP is a command and not a property edit because it NOTIFIES THE
 * ORGANIZER: irreversible, externally visible, and therefore not something a
 * bulk property update or a mapper regression may reach by accident. That is
 * the same reasoning raisin:CalendarAction carries, and the same division
 * ms-graph draws. The Google-specific twist is that there is no RSVP endpoint
 * at all — it is an events.patch of the caller's own attendee row — see
 * submit.js for why that forces a read-modify-write.
 *
 * Two more provider facts shape the mirror writes and are worth knowing before
 * reading the code:
 *   * Google has NO TRASH for events. A delete is immediate and unrecoverable,
 *     so `supports_trash` is false and the default policy is `detach`; deletes
 *     propagate only when an operator types `purge`.
 *   * Google MAILS EVERY ATTENDEE on a create, move or delete. That is
 *     irreversible and externally visible, so every write sends
 *     `sendUpdates=none` unless the mount opts in via
 *     `sync_config.send_updates` ("none" | "externalOnly" | "all").
 * Every outbound body is whatever the mount's MAPPER produced — the adapter
 * never re-derives the node -> Google field mapping, or a custom mapper pointed
 * at the same mount would silently disagree with it.
 *
 * MODULE MAP. This file is the entrypoint and the dispatch table, nothing else;
 * the implementation is split across sibling modules, loaded as ES modules by
 * the QuickJS runtime (each is a child node of this function, resolved relative
 * to this file):
 *
 *   http.js         the base URL, `calFetch`, and status -> AdapterError
 *   mount.js        which calendar, which window, the body opt-in
 *   items.js        event resource -> ExternalItem
 *   capabilities.js what this adapter declares
 *   read.js         list / get / get_content
 *   changes.js      the syncToken baseline and delta
 *   write.js        create / update / delete
 *   submit.js       the RSVP command (a submit mount)
 *   browse.js       calendarList discovery for the mount editor
 *   subscribe.js    events.watch channel lifecycle
 *
 * Token lifecycle is owned entirely by the engine: `credential.access_token` is
 * a current, decrypted token; there is NO refresh_token and no refresh logic
 * here. If a token is rejected, throw `auth_expired` and let the engine handle
 * the reconnect/refresh cycle.
 *
 * ── window + syncToken flow ────────────────────────────────────────────────
 *   full / list  → events.list bounded by a time window
 *                  (timeMin = now - window.days_back, timeMax = now + window.days_ahead).
 *                  singleEvents is NOT set, so Google returns the underlying
 *                  events: single events, recurring MASTERS carrying their RRULE,
 *                  and modified instances (exceptions) as their own records.
 *                  Unmodified occurrences are not returned and are not wanted —
 *                  they are derivable from the master.
 *   get_changes  → incremental sync via Google's opaque syncToken.
 *                  * no since_token → baseline: page a windowed list to the end
 *                    to obtain a nextSyncToken; return items:[] (the engine has
 *                    already run a full reconcile) and next_token = that token.
 *                  * with since_token → events.list?syncToken=since_token
 *                    (no timeMin/timeMax/orderBy — those invalidate a syncToken).
 *                    next_token = nextSyncToken. NEVER null — echo the prior
 *                    token when Google returns no new token.
 *                  * HTTP 410 GONE → the syncToken expired; reported as
 *                    `cursor_invalid`, so the engine drops the stored token and
 *                    full-reconciles within the same run.
 */
import { opCapabilities } from "./capabilities.js";
import { opList, opGet, opGetContent } from "./read.js";
import { opGetChanges } from "./changes.js";
import { opCreate, opUpdate, opDelete } from "./write.js";
import { opSubmit } from "./submit.js";
import { opBrowse } from "./browse.js";
import { opSubscribe, opRenew, opUnsubscribe } from "./subscribe.js";

// ---- dispatch -------------------------------------------------------------

export function handler(input) {
  var operation = input.operation;
  var params = input.params || {};
  var credential = input.credential;
  var mount = input.mount || {};

  switch (operation) {
    case "capabilities":
      return opCapabilities();
    case "list":
      return opList(credential, mount, params);
    case "get":
      return opGet(credential, mount, params);
    case "get_content":
      return opGetContent(credential, mount, params);
    case "get_changes":
      return opGetChanges(credential, mount, params);
    case "create":
      return opCreate(credential, mount, params);
    case "update":
      return opUpdate(credential, mount, params);
    case "delete":
      return opDelete(credential, mount, params);
    case "submit":
      return opSubmit(credential, mount, params);
    case "browse":
      return opBrowse(credential, mount, params);
    case "subscribe":
      return opSubscribe(credential, mount, params);
    case "renew":
      return opRenew(credential, mount, params);
    case "unsubscribe":
      return opUnsubscribe(credential, mount, params);
    default:
      throw new Error("Unsupported operation: " + operation);
  }
}
