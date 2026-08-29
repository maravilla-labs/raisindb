/**
 * WHERE a setting comes from — the merge order the engine promises — in one
 * place, because every transport in this package reads the same mount snapshot
 * and none of them may disagree about precedence.
 *
 * Shared by the IMAP path (host/port/tls/auth), the send path
 * (`email_provider`) and the Gmail push path (`username`). A second reader that
 * skipped `mount.config` would keep working against a mount configured the old
 * way and silently ignore per-connection settings — which is the failure that
 * `mount.config` was introduced to end.
 */

// Resolve the effective connection settings.
//
// `mount.config` is the engine's pre-merged view — api_config < connector
// config < CONNECTION config < sync_config — and is what carries per-connection
// settings, so one connector can serve several mailboxes on different servers.
// It is preferred; api_config and sync_config remain as fallbacks so this
// adapter keeps working against an older engine that sends neither.
//
// api_config names the mailbox `default_mailbox`; everywhere else it is `mailbox`.
export function mountSetting(mount, key) {
  var api = (mount && mount.api_config) || {};
  var sync = (mount && mount.sync_config) || {};
  var merged = (mount && mount.config) || {};
  if (merged[key] !== undefined) return merged[key];
  return sync[key] !== undefined ? sync[key] : api[key];
}

export function connConfig(mount) {
  var api = (mount && mount.api_config) || {};
  function pick(key) {
    return mountSetting(mount, key);
  }
  var mailbox = pick("mailbox");
  return {
    host: pick("host"),
    port: pick("port"),
    tls: pick("tls"),
    auth: pick("auth"),
    mailbox: mailbox !== undefined ? mailbox : api.default_mailbox,
    username: pick("username"),
  };
}
