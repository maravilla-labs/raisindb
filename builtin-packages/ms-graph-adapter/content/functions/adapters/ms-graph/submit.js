// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `submit`: issuing a COMMAND (send/reply/forward a mail, RSVP to an event)
//! rather than mirroring an object. One provider call per command, deliberately.

import { coded, enc, isEmptyObject } from "./common.js";
import { GRAPH, graphFetch, raiseForStatus } from "./http.js";
import { principal, resourceOf } from "./mount.js";
import { opUpdate } from "./write.js";

// ---- submit (the outbox) --------------------------------------------------

// ONE call per command, deliberately, and this is the load-bearing design
// decision in this operation.
//
// Graph also offers createReply/createForward, which mint a DRAFT you then PATCH
// and POST /send — three round trips. The engine's at-most-once protocol bounds
// the ambiguity of a send to ONE attempt whose outcome may be unknown; a
// three-call sequence has three such windows, and the middle one is the worst
// kind (a draft left in the mailbox that the user later finds and sends by
// hand). The single-shot actions — /sendMail, /reply, /replyAll, /forward —
// accept a full `message` body, so nothing is lost by taking them.
//
// `params` is what the engine's submit drain sends:
//   { payload: { action, body }, external_id?, idempotency_key }
// `body` is forwarded VERBATIM, exactly as opUpdate forwards `payload`: the
// MAPPER is the only authorized translator between node shape and Graph shape,
// and an adapter that rebuilt the message here would silently disagree with any
// custom mapper pointed at the same mount.
//
// `idempotency_key` is accepted and NOT sent. Graph has no idempotency header
// for any of these actions — `capabilities.supports_idempotency_key` is false
// for exactly that reason, and inventing a header would be a lie the engine
// would then rely on.
export var MAIL_ACTIONS = {
  send: null, // /sendMail, which is addressed differently from the rest
  reply: "reply",
  reply_all: "replyAll",
  forward: "forward",
};

export var RSVP_ACTIONS = {
  accept: "accept",
  decline: "decline",
  tentative: "tentativelyAccept",
};

export function submitUrl(mount, resource, action, targetId) {
  if (resource === "calendar") {
    var rsvp = RSVP_ACTIONS[action];
    if (!rsvp) {
      throw coded(
        "submit: unsupported calendar action '" + action +
          "' (expected accept, decline or tentative)",
        "config_error"
      );
    }
    if (!targetId) {
      throw coded(
        "submit: an RSVP needs the event's provider id (target_external_id)",
        "config_error"
      );
    }
    return GRAPH + principal(mount) + "/events/" + enc(targetId) + "/" + rsvp;
  }
  if (resource !== "mail") {
    throw coded(
      "submit: only the mail and calendar resources can issue commands (this mount is '" +
        resource + "')",
      "config_error"
    );
  }
  if (action === "send") {
    return GRAPH + principal(mount) + "/sendMail";
  }
  var verb = MAIL_ACTIONS[action];
  if (!verb) {
    throw coded(
      "submit: unsupported mail action '" + action +
        "' (expected send, reply, reply_all or forward)",
      "config_error"
    );
  }
  if (!targetId) {
    throw coded(
      "submit: '" + action + "' needs the provider id of the message it answers " +
        "(in_reply_to_external_id)",
      "config_error"
    );
  }
  return GRAPH + principal(mount) + "/messages/" + enc(targetId) + "/" + verb;
}

export function opSubmit(credential, mount, params) {
  params = params || {};
  var payload = params.payload || {};
  var action = payload.action;
  if (!action) {
    throw coded("submit: params.payload.action is required", "config_error");
  }
  var resource = resourceOf(mount);
  var url = submitUrl(mount, resource, action, params.external_id);
  var body = payload.body;
  if (isEmptyObject(body)) {
    throw coded("submit: refusing to issue an empty command body", "config_error");
  }

  // Every status this path diagnoses differently from a READ is claimed here.
  // What is NOT claimed (401, 429, 5xx, 408) keeps the shared mapping, which is
  // right for a send too — and note that everything the shared mapping does NOT
  // recognize reaches the engine as a plain Error, i.e. `Transient`, i.e.
  // PARKED. That is the correct default for a command, and it is the opposite
  // of the correct default for a read.
  var resp = graphFetch(credential, "POST", url, {
    headers: { "Content-Type": "application/json" },
    body: body,
    context: "submit",
    rawStatuses: [400, 403, 404],
  });

  var status = resp.status;
  var respBody = resp.body || {};
  var err = (respBody && respBody.error) || {};
  var graphCode = err.code || "";
  var graphMsg = err.message || "";

  // 404 is TERMINAL here and is NOT the `null` that `update` returns.
  //
  // On the update path a 404 means "this message moved and got a new id", and
  // the engine settles the node and waits for the delta to re-import it. There
  // is no such recovery for a command: the message being replied to is gone, so
  // this reply can never be issued as written. Returning null would park it at
  // `unknown` — i.e. tell the operator we might have sent something — which is
  // strictly false. It failed, definitively, before anything left.
  if (status === 404 || graphCode === "ErrorItemNotFound") {
    throw coded(
      "submit: the message or event this command addresses no longer exists at " +
        "Microsoft 365 (" + (graphMsg || "404") + ")",
      "config_error"
    );
  }

  // The FIRST thing a new outbox hits, and the same shape as the update path's
  // 403: the connector's OAuth scopes are read-only, so composing works and
  // sending 403s. Diagnosed rather than inherited as auth_expired, because
  // reconnecting the account with the same consent cannot fix it.
  if (status === 403) {
    throw coded(
      "submit: Microsoft Graph refused the command (403 " + (graphCode || "Forbidden") +
        "). This is almost certainly a missing SEND scope, not a stale token: add " +
        "Mail.Send (Mail.Send.Shared for a shared mailbox, Calendars.ReadWrite for " +
        "an RSVP) to the Microsoft 365 connector's OAuth scopes in the console and " +
        "RECONNECT each account — Microsoft only issues a new scope on fresh consent.",
      "config_error"
    );
  }

  if (status === 400) {
    throw coded(
      "submit: " + (graphMsg || "Microsoft Graph rejected the command (400)"),
      "config_error"
    );
  }

  raiseForStatus(resp, "submit");

  // Graph answers all of these with 202 Accepted and an EMPTY body — there is no
  // id for the message that was sent. The engine handles that: it falls back to
  // an external id derived from the command node, so the completed command is
  // still collectable by the mount's TTL cleanup. An OBJECT must still come
  // back, though; a null would be read as "the outcome is unknown".
  return {
    external_id: (respBody && respBody.id) || null,
    etag: (respBody && respBody["@odata.etag"]) || null,
  };
}
