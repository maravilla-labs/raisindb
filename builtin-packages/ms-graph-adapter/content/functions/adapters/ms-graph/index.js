/**
 * Microsoft 365 (Microsoft Graph) virtual-node adapter.
 *
 * EXPERIMENTAL / PREVIEW. Implements the frozen adapter contract
 * (docs/reference/virtual-node-adapters.md) over the Microsoft Graph v1.0 REST
 * API using the synchronous `raisin.http.fetch` binding. The sync engine invokes
 * this function directly, decrypts the account credential just before the call,
 * and materializes returned items into nodes.
 *
 * Entrypoint: handler(input) — exactly one argument.
 *   input = { operation, params, credential, mount }
 *
 * `input.mount.sync_config.resource` selects the surface:
 *   - "mail"     (default) -> {principal}/mailFolders/{id}/messages
 *   - "calendar"           -> {principal}/calendars/{calId}/events, and
 *                             {principal}/calendarView/delta for the PRIMARY
 *                             calendar only (v1.0 has no per-calendar delta, so
 *                             a secondary/shared calendar reports
 *                             supports_changes:false and full-reconciles)
 *   - "files"              -> {drive}/root/children + {drive}/root/delta
 *
 * `{principal}` is `/me` by default and `/users/{upn}` when the mount or
 * connection names a `principal` — that is what makes SHARED MAILBOXES work.
 * `{drive}` additionally honours `drive_scope` (me | user | site), which is what
 * makes a SharePoint document library mountable through the same `files`
 * resource and the same mapper. See `principal()` / `driveBase()` below.
 *
 * Push (EXPERIMENTAL): mail/calendar/files are all push-capable. The engine calls
 * `subscribe`/`renew`/`unsubscribe` to manage a Microsoft Graph subscription whose
 * notifications are a pure invalidation signal — the engine re-runs `get_changes`.
 * The Graph validationToken handshake is echoed by the RaisinDB notifications
 * endpoint, NOT here; `clientState` is the per-subscription secret the endpoint
 * verifies.
 *
 * Token lifecycle is owned entirely by the engine: `credential.access_token` is
 * a current, decrypted bearer token; there is NO refresh_token and no refresh
 * logic here. If a token is rejected, throw `auth_expired` and let the engine
 * handle the reconnect/refresh cycle.
 *
 * Writes are declared PER RESOURCE, never once for the whole adapter, and only
 * for what is implemented:
 *   - mail     -> `update` (a state PATCH — today the read flag) + `submit`
 *   - calendar -> the full mirror set (`create`/`update`/`delete`) + `submit`
 *                 (RSVP)
 *   - files    -> the full mirror set (`create`/`update`/`delete`) PLUS bytes:
 *                 `accepts_content` makes the engine send the file, and
 *                 anything over Microsoft's 4 MiB simple-PUT ceiling is
 *                 answered with an upload session the ENGINE streams to,
 *                 followed by a `finalize_upload` call back into here
 * A capability with no implementation behind it is how a mount resolves to a
 * mode that throws at drain time, after the engine has claimed its candidates —
 * so silence here is deliberate, not an oversight.
 *
 * Every outbound body is whatever the mount's MAPPER produced — the adapter
 * never re-derives the node -> Graph field mapping (§6a of the write-path
 * design): a custom mapper that reshapes nodes must be the one that reshapes
 * them back.
 *
 * MODULE MAP. This file is the entrypoint and the dispatch table, nothing else.
 * The implementation is split across sibling modules, loaded as ES modules by
 * the QuickJS runtime (each sibling is a child node of this function, resolved
 * relative to this file):
 *
 *   common.js       coded errors, URL encoding — no Graph knowledge, no I/O
 *   http.js         the base URL, `graphFetch`, and status -> AdapterError
 *   mount.js        which surface/principal/calendar a mount addresses, $select
 *   mail.js         address formatting, mail metadata, attachment enrichment
 *   calendar.js     patternedRecurrence -> RFC 5545, event metadata
 *   items.js        provider object -> ExternalItem, per resource
 *   capabilities.js what this adapter declares, per resource
 *   read.js         list / get / get_content
 *   changes.js      the delta feed and the one-node-per-series collapse
 *   write-common.js the etag header, the receipt, status -> write diagnosis
 *   write.js        create / update / delete, dispatched per resource
 *   drive.js        the OneDrive/SharePoint write path (driveItems)
 *   drive-upload.js its byte half: the size fork, sessions, finalize_upload
 *   submit.js       send, reply, forward, RSVP
 *   subscribe.js    Graph push subscriptions
 *   browse.js       discovery for the mount editor (never called during sync)
 */

import { opCapabilities } from "./capabilities.js";
import { opList, opGet, opGetContent } from "./read.js";
import { opCreate, opUpdate, opDelete } from "./write.js";
import { opFinalizeUpload } from "./drive-upload.js";
import { opSubmit } from "./submit.js";
import { opGetChanges } from "./changes.js";
import { opSubscribe, opRenew, opUnsubscribe } from "./subscribe.js";
import { opBrowse } from "./browse.js";

// ---- dispatch -------------------------------------------------------------

export function handler(input) {
  var operation = input.operation;
  var params = input.params || {};
  var credential = input.credential;
  var mount = input.mount || {};

  switch (operation) {
    case "capabilities":
      return opCapabilities(input.mount || {});
    case "list":
      return opList(credential, mount, params);
    case "get":
      return opGet(credential, mount, params);
    case "get_content":
      return opGetContent(credential, mount, params);
    case "create":
      return opCreate(credential, mount, params);
    case "update":
      return opUpdate(credential, mount, params);
    case "delete":
      return opDelete(credential, mount, params);
    // The engine streamed the bytes itself and is handing back the provider's
    // final response. No credential is involved: the upload URL carried its own
    // authorization, and all that is left is reading a driveItem's id and etag
    // out of a body — provider-shaped parsing, which is why it comes back here
    // instead of being done in Rust.
    case "finalize_upload":
      return opFinalizeUpload(mount, params);
    case "submit":
      return opSubmit(credential, mount, params);
    case "get_changes":
      return opGetChanges(credential, mount, params);
    case "subscribe":
      return opSubscribe(credential, mount, params);
    case "renew":
      return opRenew(credential, params);
    case "unsubscribe":
      return opUnsubscribe(credential, params);
    case "browse":
      return opBrowse(credential, mount, params);
    default:
      throw new Error("Unsupported operation: " + operation);
  }
}
