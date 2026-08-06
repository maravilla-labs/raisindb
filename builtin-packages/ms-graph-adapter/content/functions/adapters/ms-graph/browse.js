// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Discovery for the mount editor. Never called during sync.

import { coded, enc, encSiteId } from "./common.js";
import { GRAPH, graphFetch } from "./http.js";
import { driveBase, principal, resourceOf } from "./mount.js";

// ---- browse (discovery for the mount editor) ------------------------------

// Never called during sync. See §2.10 of the adapter contract: this exists so an
// operator picks a folder/calendar/site/library instead of pasting a Graph id.

export function browseItem(id, name, kind, hasChildren, hint) {
  return {
    id: id,
    name: name || id,
    kind: kind,
    has_children: hasChildren === true,
    hint: hint || null,
  };
}

// One Graph page -> BrowseItem[]. `map` turns a Graph entity into a BrowseItem.
export function browsePage(credential, url, map) {
  var resp = graphFetch(credential, "GET", url, { context: "browse" });
  var body = resp.body || {};
  var values = body.value || [];
  var items = [];
  for (var i = 0; i < values.length; i++) {
    var item = map(values[i]);
    if (item) items.push(item);
  }
  return { items: items, next_cursor: body["@odata.nextLink"] || null };
}

export function browseLimit(params) {
  return params && params.limit && params.limit > 0
    ? Math.min(params.limit, 200)
    : 50;
}

// `startswith` search over the directory, shared by the `mailbox` and `user`
// kinds. Single quotes are doubled per OData escaping.
export function directoryFilter(params) {
  if (!params || !params.query || !params.query.trim()) return "";
  var term = params.query.trim().split("'").join("''");
  return "&$filter=" + enc(
    "startswith(displayName,'" + term + "') or startswith(mail,'" + term + "')"
  );
}

// One mapper per browse kind, so the first page and every later page agree.
export function browseMapper(kind) {
  if (kind === "folder") {
    return function (v) {
      return browseItem(
        v.id, v.displayName, "folder",
        (v.childFolderCount || 0) > 0,
        v.totalItemCount != null ? v.totalItemCount + " items" : null
      );
    };
  }
  if (kind === "calendar") {
    return function (v) {
      var owner = v.owner ? v.owner.address : null;
      // `canShare: false` on a calendar you did not create is Graph's marker for
      // "shared with me" — there is no `isShared` property in v1.0.
      var hint = v.isDefaultCalendar ? "primary" : owner;
      if (v.canShare === false && !v.isDefaultCalendar) hint = "shared · " + (owner || "");
      return browseItem(v.id, v.name, "calendar", false, hint);
    };
  }
  if (kind === "mailbox" || kind === "user") {
    return function (v) {
      var addr = v.mail || v.userPrincipalName;
      if (!addr) return null;
      // A shared mailbox is an unlicensed, sign-in-blocked directory object, so
      // `accountEnabled: false` is the only cheap signal v1.0 offers. It is a
      // HEURISTIC, not proof — `mailboxSettings/userPurpose` is authoritative but
      // needs APPLICATION permission to read for anyone but the signed-in user,
      // which delegated OAuth cannot do. Labelled as "likely" for that reason.
      var hint = v.accountEnabled === false ? "likely shared mailbox" : addr;
      return browseItem(addr, v.displayName || addr, kind, kind === "user", hint);
    };
  }
  if (kind === "room") {
    return function (v) {
      var addr = v.emailAddress;
      if (!addr) return null;
      // Key on the SMTP address, never `place.id`, which Microsoft documents as
      // NOT immutable. The address also drops straight into the `/users/{addr}`
      // principal a room mailbox is addressed by.
      return browseItem(addr, v.displayName || addr, "room", false, v.building || addr);
    };
  }
  return function (v) {
    return browseItem(v.id, v.displayName || v.name, kind, false, null);
  };
}

export function opBrowse(credential, mount, params) {
  params = params || {};
  // A cursor is a full Graph nextLink; the kind is already baked into it.
  if (params.cursor) {
    // Map with the SAME function the first page used. The generic mapper here
    // fell back to `v.id`, so page two of a `mailbox` listing returned directory
    // object GUIDs where page one returned SMTP addresses — and the address is
    // what gets written into `principal`, so a mailbox picked from page two
    // produced a mount that could never resolve.
    return browsePage(credential, params.cursor, browseMapper(params.kind || "item"));
  }

  var top = browseLimit(params);
  var parent = params.parent_id || null;
  var kind = params.kind || (resourceOf(mount) === "calendar" ? "calendar" : "folder");

  // Mail folders. Hierarchical: childFolderCount drives the expander, and
  // `parent_id` walks into a subfolder.
  if (kind === "folder") {
    var base = GRAPH + principal(mount) + "/mailFolders";
    var url = parent
      ? base + "/" + enc(parent) + "/childFolders?$top=" + top
      : base + "?$top=" + top;
    return browsePage(credential, url, function (v) {
      return browseItem(
        v.id,
        v.displayName,
        "folder",
        (v.childFolderCount || 0) > 0,
        v.totalItemCount != null ? v.totalItemCount + " items" : null
      );
    });
  }

  // Calendars, either the mount principal's or — with `parent_id` set to an SMTP
  // address — another mailbox's.
  //
  // The second form matters because a shared PRIMARY calendar never appears in
  // `/me/calendars`. Only accepted shares of SECONDARY calendars are copied into
  // the recipient's mailbox; "Alex shared his main calendar with me", which is
  // the common case, is reachable only as `/users/{alex}/calendars`. Browsing
  // `/me` alone therefore hid exactly the calendars an admin most wants to mount.
  //
  // The emitted id is the calendar id, and the caller must pair it with
  // `principal = parent_id`: calendar ids are MAILBOX-SCOPED, so an id from one
  // mailbox addressed under another simply errors.
  if (kind === "calendar") {
    var calBase = parent ? "/users/" + enc(parent) : principal(mount);
    return browsePage(
      credential,
      GRAPH + calBase + "/calendars?$top=" + top,
      browseMapper("calendar")
    );
  }

  // People, as step one of picking someone else's calendar. `has_children` is
  // true so the picker drills into `kind: "calendar"` with `parent_id` = address.
  //
  // Same caveat as `mailbox` below: this is the DIRECTORY, not an access list.
  if (kind === "user") {
    return browsePage(
      credential,
      GRAPH + "/users?$select=id,displayName,mail,userPrincipalName,accountEnabled&$top=" +
        top + directoryFilter(params),
      browseMapper("user")
    );
  }

  // Room and equipment mailboxes. `/places` is v1.0 and needs Place.Read.All.
  //
  // Deliberately NOT `findRooms`, which is beta-only and caps at 100.
  if (kind === "room") {
    return browsePage(
      credential,
      GRAPH + "/places/microsoft.graph.room?$top=" + top,
      browseMapper("room")
    );
  }

  // SharePoint sites. Graph requires a search term to enumerate; `*` is the
  // documented "all sites" wildcard.
  if (kind === "site") {
    var q = params.query && params.query.trim() ? params.query.trim() : "*";
    return browsePage(
      credential,
      GRAPH + "/sites?search=" + enc(q) + "&$top=" + top,
      function (v) {
        // The COMPOSITE id (hostname,siteGuid,webGuid) is what a mount needs —
        // the short guid alone will not resolve /sites/{id}/drives.
        return browseItem(v.id, v.displayName || v.name, "site", true, v.webUrl || null);
      }
    );
  }

  // Document libraries of a site (parent_id) or a user's drives.
  if (kind === "drive") {
    var owner = parent
      ? "/sites/" + encSiteId(parent)
      : principal(mount) === "/me"
        ? "/me"
        : principal(mount);
    return browsePage(credential, GRAPH + owner + "/drives?$top=" + top, function (v) {
      return browseItem(v.id, v.name, "drive", true, v.driveType || null);
    });
  }

  // Folders inside a drive. Files are omitted deliberately: this picker chooses
  // a mount ROOT, and a file is never one.
  if (kind === "driveItem") {
    var container = parent
      ? driveBase(mount) + "/items/" + enc(parent)
      : driveBase(mount) + "/root";
    return browsePage(
      credential,
      GRAPH + container + "/children?$top=" + top,
      function (v) {
        if (!v.folder) return null;
        return browseItem(
          v.id,
          v.name,
          "driveItem",
          (v.folder.childCount || 0) > 0,
          v.folder.childCount != null ? v.folder.childCount + " items" : null
        );
      }
    );
  }

  // Directory users, for picking a shared mailbox or another user's OneDrive.
  //
  // NOT a permission-accurate list: Graph has no delegated API that returns
  // "the mailboxes this account may open", so this enumerates the DIRECTORY
  // (requires User.ReadBasic.All) and a listed mailbox can still fail at sync
  // time. The console therefore keeps manual entry, and Test connection is what
  // actually proves access. Do not "improve" this into an access list.
  if (kind === "mailbox") {
    return browsePage(
      credential,
      GRAPH + "/users?$select=id,displayName,mail,userPrincipalName,accountEnabled&$top=" +
        top + directoryFilter(params),
      browseMapper("mailbox")
    );
  }


  throw coded("browse: unsupported kind '" + kind + "'", "config_error");
}
