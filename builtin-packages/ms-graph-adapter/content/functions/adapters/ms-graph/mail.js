// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Mail: address formatting, the `metadata` bag one message becomes, and the
//! attachment enrichment pass.

import { enc } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { includeAttachments, outlookHeaders, principal } from "./mount.js";

// ---- address formatting ---------------------------------------------------

export function fmtAddr(recip) {
  if (!recip || !recip.emailAddress) return null;
  var e = recip.emailAddress;
  if (e.name && e.address) return e.name + " <" + e.address + ">";
  return e.address || e.name || null;
}

export function fmtAddrList(list) {
  if (!list || !list.length) return null;
  var out = [];
  for (var i = 0; i < list.length; i++) {
    var a = fmtAddr(list[i]);
    if (a) out.push(a);
  }
  return out.length ? out.join(", ") : null;
}

// ---- ExternalItem builders ------------------------------------------------

// Bare address with the display name stripped: `from` carries whatever name the
// sender currently uses, which changes over time and makes it a poor grouping
// key. The mapper indexes this one separately for GROUP BY / equality.
export function bareAddr(recip) {
  return recip && recip.emailAddress ? recip.emailAddress.address || null : null;
}

export function mailMeta(v) {
  var meta = {
    subject: v.subject || null,
    from: fmtAddr(v.from),
    from_address: bareAddr(v.from),
    to: fmtAddrList(v.toRecipients),
    cc: fmtAddrList(v.ccRecipients),
    bcc: fmtAddrList(v.bccRecipients),
    reply_to: fmtAddrList(v.replyTo),
    // `date` stays whichever applies to THIS copy and is the canonical ORDER BY
    // column; the two halves travel alongside it unambiguously, because "no
    // received_at" is how an outgoing copy is recognised.
    date: v.receivedDateTime || v.sentDateTime || null,
    received_at: v.receivedDateTime || null,
    sent_at: v.sentDateTime || null,
    is_draft: v.isDraft === true,
    snippet: v.bodyPreview || null,
    unread: v.isRead === false,
    has_attachments: v.hasAttachments === true,
    importance: v.importance || null,
    // Outlook categories and Gmail labels are the same concept; the global
    // nodetype carries one `labels` array for both so a consumer never has to
    // branch on provider to read a label.
    labels: v.categories && v.categories.length ? v.categories : null,
    conversation_id: v.conversationId || null,
    internet_message_id: v.internetMessageId || null,
    web_url: v.webLink || null,
  };
  // Present only when the mount opted in; `body` is absent from $select
  // otherwise, and an absent key is what tells the mapper to leave the property
  // unset rather than write an empty string over a previously synced body.
  //
  // Split at the SOURCE rather than carried as `body` + `body_type`: Graph tells
  // us which representation it sent, and a single column whose meaning depends
  // on a sibling column cannot be searched or replied to without branching.
  if (v.body && typeof v.body.content === "string") {
    if (v.body.contentType === "html") {
      meta.body_html = v.body.content;
    } else {
      meta.body_text = v.body.content;
    }
  }
  return meta;
}

// Attachment METADATA for one message — never bytes.
//
// `$select` omits `contentBytes` on purpose: including it inlines every
// attachment as base64 into the sync response, which is the exact payload
// explosion `include_body` already exists to avoid, times every attachment.
// The engine materializes these as raisin:Asset children and fetches a blob
// through `get_content` only when something opens it.
//
// Attach `metadata.attachments` to every mail item, or `attachments_unknown`
// when the listing could not be read. Applied in one helper called from both the
// list and the delta path, because a mail whose attachments the full walk
// materialized and the delta did not would have those child nodes reconciled
// away on the next full run and recreated on the one after.
//
// NOT gated on `has_attachments`. Microsoft documents that flag as excluding
// INLINE attachments, so every HTML newsletter and every pasted screenshot
// reported `false` and its images were never imported — and because the walk and
// the delta shared the gate, neither path could ever repair the other. The gate
// bought one saved request per plain message; it cost every inline image in the
// mailbox. `include_attachments` is already an opt-in whose documented cost is
// one request per message, so paying it uniformly is the honest reading of the
// flag.
//
// A FAILURE IS NOT AN EMPTY LIST. Returning null on error let the engine read a
// throttled listing as "this message has no attachments" — so the Asset children
// it had already imported fell out of the walk's `seen` set and reconcile
// DELETED them, permanently, since the message's own etag had not moved. The
// distinction now travels explicitly: `attachments` present means we know, and
// `children_unknown` — an engine-level contract key, not a mail one — means we
// do not, so the engine keeps the children it already has instead of pruning
// them.
export function enrichAttachments(credential, mount, items) {
  if (!includeAttachments(mount)) return items;
  for (var i = 0; i < items.length; i++) {
    var meta = items[i] && items[i].metadata;
    if (!meta) continue;
    var list = mailAttachments(credential, mount, items[i].external_id);
    if (list) meta.attachments = list;
    else meta.children_unknown = true;
  }
  return items;
}

// The attachment listing for one message, or null when the message itself is
// gone. Anything else THROWS.
//
// A 404 is the one error that genuinely means "no attachments to list": the
// message was deleted between the page fetch and this call, and the walk will
// reconcile it away anyway. Every other failure — 429, 5xx, an expired token —
// is a statement about the request, not about the message, and must reach the
// engine so it backs off rather than acting on a listing it never got.
// What every attachment kind has. `contentBytes` is deliberately absent (see
// above); everything here is on the BASE `microsoft.graph.attachment` type and
// is therefore always selectable.
var ATTACHMENT_SELECT = "id,name,contentType,size,isInline";

// `contentId` is NOT on the base type — it belongs to the derived
// `microsoft.graph.fileAttachment`, so selecting it unqualified makes Graph
// reject the WHOLE request:
//
//   Could not find a property named 'contentId' on type 'microsoft.graph.attachment'
//
// which arrives as a 400, is classified `config_error`, and stops the mount —
// every message, not just the ones with attachments. OData addresses a derived
// property by casting, which is what this does.
var ATTACHMENT_SELECT_CID =
  ATTACHMENT_SELECT + ",microsoft.graph.fileAttachment/contentId";

function attachmentsUrl(mount, messageId, select) {
  return (
    GRAPH +
    principal(mount) +
    "/messages/" +
    enc(messageId) +
    "/attachments?$select=" +
    select
  );
}

// Does this 400 mean "that select is not valid here"?
//
// Narrow on purpose: a 400 about anything else must still surface. The cast
// above is the documented form, but a tenant or a future Graph revision that
// refuses it should cost inline `cid:` resolution, not the whole mailbox.
function isSelectRejection(resp) {
  if (!resp || resp.status !== 400) return false;
  var body = resp.body || {};
  var message = (body.error && body.error.message) || "";
  return /select|contentId|expand/i.test(String(message));
}

export function mailAttachments(credential, mount, messageId) {
  var resp = graphFetch(
    credential,
    "GET",
    attachmentsUrl(mount, messageId, ATTACHMENT_SELECT_CID),
    {
      context: "list attachments",
      headers: outlookHeaders(mount),
      // 404 stays the caller's (the message vanished); 400 is taken raw so the
      // select rejection can be told apart from every other bad request —
      // `raiseForStatus` would otherwise map it to config_error and stop the
      // mount before this function saw it.
      rawStatusOk: true,
      rawStatuses: [400],
    }
  );
  // Retried WITHOUT the cast rather than failing the mount. The cost is
  // `content_id: null`, so an inline image still imports as an Asset child and
  // only its `cid:` reference in the body goes unresolved.
  if (isSelectRejection(resp)) {
    resp = graphFetch(
      credential,
      "GET",
      attachmentsUrl(mount, messageId, ATTACHMENT_SELECT),
      {
        context: "list attachments",
        headers: outlookHeaders(mount),
        rawStatusOk: true,
      }
    );
  }
  if (resp.status === 404) return null;
  // Every non-2xx that survived the retry is mapped here as before, including a
  // 400 about something other than the select.
  raiseForStatus(resp, "list attachments");
  var list = (resp.body && resp.body.value) || [];
  var out = [];
  for (var i = 0; i < list.length; i++) {
    var a = list[i];
    if (!a || !a.id) continue;
    out.push({
      external_id: a.id,
      name: a.name || a.id,
      mime_type: a.contentType || null,
      size: a.size != null ? a.size : null,
      inline: a.isInline === true,
      content_id: a.contentId || null,
    });
  }
  return out;
}
