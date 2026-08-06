// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Mail: address formatting, the `metadata` bag one message becomes, and the
//! attachment enrichment pass.

import { enc } from "./common.js";
import { GRAPH, graphFetch } from "./http.js";
import { includeAttachments, principal } from "./mount.js";

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
// A failure here is swallowed: the MESSAGE is real content and must still
// sync. Losing its attachment list costs the child nodes, not the mail.
// Attach `metadata.attachments` to every mail item that reports having any.
//
// Driven by `has_attachments`, so a mailbox of plain messages costs ZERO extra
// requests — the flag comes free in the projection. Applied in one helper called
// from both the list and the delta path, because a mail whose attachments the
// full walk materialized and the delta did not would have those child nodes
// reconciled away on the next full run and recreated on the one after.
export function enrichAttachments(credential, mount, items) {
  if (!includeAttachments(mount)) return items;
  for (var i = 0; i < items.length; i++) {
    var meta = items[i] && items[i].metadata;
    if (!meta || meta.has_attachments !== true) continue;
    var list = mailAttachments(credential, mount, items[i].external_id);
    if (list) meta.attachments = list;
  }
  return items;
}

export function mailAttachments(credential, mount, messageId) {
  try {
    var resp = graphFetch(
      credential,
      "GET",
      GRAPH +
        principal(mount) +
        "/messages/" +
        enc(messageId) +
        "/attachments?$select=id,name,contentType,size,isInline,contentId",
      { context: "list attachments" }
    );
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
  } catch (e) {
    return null;
  }
}
