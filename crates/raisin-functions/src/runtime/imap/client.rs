// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Real IMAP client operations over TLS using `async-imap`.
//!
//! Flow: TCP connect -> TLS handshake (tokio-rustls) -> LOGIN -> SELECT ->
//! UID SEARCH / UID FETCH. Every public entry point is bounded by an overall
//! per-operation timeout and always attempts a clean LOGOUT. Protocol failures
//! surface as typed [`ImapError`]s (never panics); credentials are never logged.

use std::sync::Arc;
use std::time::Duration;

use async_imap::error::Error as ImapProtoError;
use async_imap::types::Flag;
use futures::StreamExt;
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use super::parse::{detail_from_raw, flag_to_string, summary_from_fetch};
use super::transport::{ImapStream, XOAuth2};
use super::{
    FetchSinceResult, ImapAuth, ImapConn, ImapError, MailboxInfo, MessageDetail, OP_TIMEOUT_SECS,
};

type Session = async_imap::Session<ImapStream>;

/// Build a rustls client config trusting the webpki root bundle, pinned to the
/// aws-lc-rs provider so the process-level default is never ambiguous.
fn tls_config() -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("aws-lc-rs supports the default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    Arc::new(config)
}

/// Classify a protocol error message into a rate-limit vs. generic protocol
/// error. Login failures are classified as auth errors by the caller.
fn is_rate_limited(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("throttl")
        || m.contains("rate limit")
        || m.contains("too many")
        || m.contains("try again")
        || m.contains("[limit]")
        || m.contains("overquota")
}

fn map_imap_err(e: ImapProtoError) -> ImapError {
    let msg = e.to_string();
    if is_rate_limited(&msg) {
        ImapError::RateLimited(msg)
    } else {
        ImapError::Protocol(msg)
    }
}

fn map_login_err(e: ImapProtoError) -> ImapError {
    let msg = e.to_string();
    if is_rate_limited(&msg) {
        ImapError::RateLimited(msg)
    } else {
        // A rejected LOGIN is an auth problem the adapter maps to auth_expired.
        ImapError::Auth(msg)
    }
}

fn attr_to_string(a: &async_imap::imap_proto::NameAttribute<'_>) -> String {
    use async_imap::imap_proto::NameAttribute as N;
    match a {
        // Server-defined attributes (e.g. `\HasChildren`) arrive as Extension.
        N::Extension(s) => s.to_string(),
        other => format!("\\{other:?}"),
    }
}

/// Open the transport (TLS or plaintext) and authenticate, returning a session.
async fn connect_login(conn: &ImapConn) -> Result<Session, ImapError> {
    let tcp = TcpStream::connect((conn.host.as_str(), conn.port))
        .await
        .map_err(|e| ImapError::Protocol(format!("tcp connect failed: {e}")))?;

    let stream = if conn.tls {
        let server_name = ServerName::try_from(conn.host.clone())
            .map_err(|e| ImapError::Config(format!("invalid server name: {e}")))?;
        let tls = TlsConnector::from(tls_config())
            .connect(server_name, tcp)
            .await
            .map_err(|e| ImapError::Protocol(format!("tls handshake failed: {e}")))?;
        ImapStream::Tls(Box::new(tls))
    } else {
        ImapStream::Plain(tcp)
    };

    let mut client = async_imap::Client::new(stream);
    // Consume the server greeting before authenticating.
    client
        .read_response()
        .await
        .map_err(|e| ImapError::Protocol(format!("no server greeting: {e}")))?
        .ok_or_else(|| ImapError::Protocol("connection closed before greeting".to_string()))?;

    match conn.auth {
        ImapAuth::Password => client
            .login(&conn.username, &conn.password)
            .await
            .map_err(|(e, _client)| map_login_err(e)),
        ImapAuth::Xoauth2 => client
            .authenticate("XOAUTH2", XOAuth2::new(&conn.username, &conn.password))
            .await
            .map_err(|(e, _client)| map_login_err(e)),
    }
}

/// Fetch messages with UID greater than `since_uid` (bounded by `limit`).
pub async fn fetch_since(
    conn: &ImapConn,
    since_uid: u32,
    limit: usize,
    mailbox: &str,
) -> Result<FetchSinceResult, ImapError> {
    with_timeout(fetch_since_inner(conn, since_uid, limit, mailbox)).await
}

async fn fetch_since_inner(
    conn: &ImapConn,
    since_uid: u32,
    limit: usize,
    mailbox: &str,
) -> Result<FetchSinceResult, ImapError> {
    let mut session = connect_login(conn).await?;
    let mbox = session.select(mailbox).await.map_err(map_imap_err)?;
    let uidvalidity = mbox.uid_validity.unwrap_or(0);

    // `UID n:*` can echo the highest UID even when n exceeds it, so filter.
    let query = format!("UID {}:*", since_uid.saturating_add(1));
    let found = session.uid_search(query).await.map_err(map_imap_err)?;
    let mut uids: Vec<u32> = found.into_iter().filter(|u| *u > since_uid).collect();
    uids.sort_unstable();
    if uids.len() > limit {
        // Keep the newest `limit` UIDs.
        uids = uids.split_off(uids.len() - limit);
    }

    let mut messages = Vec::new();
    let mut highest = since_uid;
    if !uids.is_empty() {
        let set = uids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut stream = session
            .uid_fetch(set, "(UID FLAGS ENVELOPE)")
            .await
            .map_err(map_imap_err)?;
        while let Some(item) = stream.next().await {
            let fetch = item.map_err(map_imap_err)?;
            if let Some(summary) = summary_from_fetch(&fetch) {
                highest = highest.max(summary.uid);
                messages.push(summary);
            }
        }
        drop(stream);
    }

    let _ = session.logout().await;
    messages.sort_by_key(|m| m.uid);

    Ok(FetchSinceResult {
        messages,
        highest_uid: highest,
        uidvalidity,
    })
}

/// List all mailboxes (folders) on the account.
pub async fn list_mailboxes(conn: &ImapConn) -> Result<Vec<MailboxInfo>, ImapError> {
    with_timeout(list_mailboxes_inner(conn)).await
}

async fn list_mailboxes_inner(conn: &ImapConn) -> Result<Vec<MailboxInfo>, ImapError> {
    let mut session = connect_login(conn).await?;
    let mut out = Vec::new();
    {
        let mut stream = session.list(None, Some("*")).await.map_err(map_imap_err)?;
        while let Some(item) = stream.next().await {
            let name = item.map_err(map_imap_err)?;
            let path = name.name().to_string();
            let leaf = match name.delimiter() {
                Some(d) if !d.is_empty() => path.rsplit(d).next().unwrap_or(&path).to_string(),
                _ => path.clone(),
            };
            let flags = name.attributes().iter().map(attr_to_string).collect();
            out.push(MailboxInfo {
                name: leaf,
                path,
                flags,
            });
        }
    }
    let _ = session.logout().await;
    Ok(out)
}

/// Fetch one full message by UID (headers, text, html, flags).
pub async fn fetch_message(
    conn: &ImapConn,
    uid: u32,
    mailbox: &str,
) -> Result<MessageDetail, ImapError> {
    with_timeout(fetch_message_inner(conn, uid, mailbox)).await
}

async fn fetch_message_inner(
    conn: &ImapConn,
    uid: u32,
    mailbox: &str,
) -> Result<MessageDetail, ImapError> {
    let mut session = connect_login(conn).await?;
    session.select(mailbox).await.map_err(map_imap_err)?;

    let mut detail: Option<MessageDetail> = None;
    {
        let mut stream = session
            .uid_fetch(uid.to_string(), "(UID FLAGS BODY.PEEK[])")
            .await
            .map_err(map_imap_err)?;
        while let Some(item) = stream.next().await {
            let fetch = item.map_err(map_imap_err)?;
            let flags: Vec<String> = fetch
                .flags()
                .map(|f: Flag<'_>| flag_to_string(&f))
                .collect();
            let raw = fetch.body().or_else(|| fetch.text()).unwrap_or(&[]);
            detail = Some(detail_from_raw(uid, flags, raw));
        }
    }
    let _ = session.logout().await;

    detail.ok_or_else(|| ImapError::Protocol(format!("message uid {uid} not found")))
}

async fn with_timeout<T>(
    fut: impl std::future::Future<Output = Result<T, ImapError>>,
) -> Result<T, ImapError> {
    match tokio::time::timeout(Duration::from_secs(OP_TIMEOUT_SECS), fut).await {
        Ok(res) => res,
        Err(_) => Err(ImapError::Timeout),
    }
}
