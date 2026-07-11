// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Native IMAP operation implementations for RaisinFunctionApi.
//!
//! Every operation enforces the function's network policy on the connection's
//! `host:port` BEFORE opening any socket, then delegates to the pure-Rust IMAP
//! client in `runtime::imap`. Credentials are never logged.

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;
use crate::runtime::imap::{self, ImapConn, DEFAULT_FETCH_LIMIT, MAX_FETCH_LIMIT};

impl RaisinFunctionApi {
    /// Parse the connection descriptor and authorize it against the network
    /// policy. Returns the parsed [`ImapConn`] only if `imaps://host:port`
    /// (or `imap://` when `tls=false`) is allowed. This runs before any socket
    /// is opened so a disallowed host can never be contacted.
    fn authorize_imap(&self, conn: Value) -> Result<ImapConn> {
        let conn = ImapConn::from_value(&conn).map_err(raisin_error::Error::from)?;
        let policy_url = conn.policy_url();
        if !self.is_url_allowed(&policy_url) {
            // Do not echo the URL host into the error at trace-sensitive sites;
            // the policy_url contains no secret, only host:port.
            tracing::debug!(
                target = %policy_url,
                "imap connection blocked by network policy"
            );
            return Err(raisin_error::Error::PermissionDenied(format!(
                "[imap:policy_denied] IMAP endpoint not allowed by network policy: {policy_url}"
            )));
        }
        Ok(conn)
    }

    pub(crate) async fn impl_imap_fetch_since(
        &self,
        conn: Value,
        since_uid: i64,
        opts: Option<Value>,
    ) -> Result<Value> {
        let conn = self.authorize_imap(conn)?;
        let opts = opts.unwrap_or(Value::Null);
        let mailbox = opt_mailbox(&opts);
        let limit = opts
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, MAX_FETCH_LIMIT))
            .unwrap_or(DEFAULT_FETCH_LIMIT);
        let since = since_uid.max(0) as u32;

        let result = imap::fetch_since(&conn, since, limit, &mailbox)
            .await
            .map_err(raisin_error::Error::from)?;
        serde_json::to_value(result)
            .map_err(|e| raisin_error::Error::Internal(format!("imap serialize failed: {e}")))
    }

    pub(crate) async fn impl_imap_list_mailboxes(&self, conn: Value) -> Result<Value> {
        let conn = self.authorize_imap(conn)?;
        let result = imap::list_mailboxes(&conn)
            .await
            .map_err(raisin_error::Error::from)?;
        serde_json::to_value(result)
            .map_err(|e| raisin_error::Error::Internal(format!("imap serialize failed: {e}")))
    }

    pub(crate) async fn impl_imap_fetch_message(
        &self,
        conn: Value,
        uid: i64,
        opts: Option<Value>,
    ) -> Result<Value> {
        let conn = self.authorize_imap(conn)?;
        let mailbox = opt_mailbox(&opts.unwrap_or(Value::Null));
        let uid = uid.max(0) as u32;

        let result = imap::fetch_message(&conn, uid, &mailbox)
            .await
            .map_err(raisin_error::Error::from)?;
        serde_json::to_value(result)
            .map_err(|e| raisin_error::Error::Internal(format!("imap serialize failed: {e}")))
    }
}

/// Extract `opts.mailbox`, defaulting to `INBOX`.
fn opt_mailbox(opts: &Value) -> String {
    opts.get("mailbox")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("INBOX")
        .to_string()
}
