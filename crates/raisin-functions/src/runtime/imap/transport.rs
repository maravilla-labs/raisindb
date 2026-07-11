// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! IMAP transport + SASL authentication primitives.
//!
//! [`ImapStream`] lets a session run over either implicit TLS or a plaintext
//! socket behind one type, and [`XOAuth2`] implements the SASL `XOAUTH2`
//! mechanism (RFC 7628) for OAuth2 bearer-token auth.

use std::pin::Pin;
use std::task::{Context, Poll};

use async_imap::Authenticator;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// Transport for an IMAP session: either implicit TLS or a plaintext socket.
/// Both variants implement tokio's async IO so `async-imap` is agnostic to
/// which is in use. The large `TlsStream` is boxed to keep the enum small.
#[derive(Debug)]
pub(super) enum ImapStream {
    Tls(Box<TlsStream<TcpStream>>),
    Plain(TcpStream),
}

impl AsyncRead for ImapStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
            ImapStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ImapStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            ImapStream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
            ImapStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
            ImapStream::Plain(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            ImapStream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
            ImapStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// SASL `XOAUTH2` authenticator. Emits the RFC 7628 initial client response
/// (`user=<user>^Aauth=Bearer <token>^A^A`) on the first challenge and an empty
/// response on any subsequent challenge (the server's base64 error blob).
pub(super) struct XOAuth2 {
    initial: Option<Vec<u8>>,
}

impl XOAuth2 {
    pub(super) fn new(user: &str, token: &str) -> Self {
        let init = format!("user={user}\x01auth=Bearer {token}\x01\x01").into_bytes();
        Self {
            initial: Some(init),
        }
    }
}

impl Authenticator for XOAuth2 {
    type Response = Vec<u8>;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        self.initial.take().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoauth2_emits_rfc7628_initial_response_then_empty() {
        let mut auth = XOAuth2::new("alice@example.com", "ya29.TOKEN");
        // First challenge yields the SASL init string.
        assert_eq!(
            auth.process(b""),
            b"user=alice@example.com\x01auth=Bearer ya29.TOKEN\x01\x01".to_vec()
        );
        // Any subsequent challenge (e.g. an error blob) yields an empty response.
        assert!(auth.process(b"eyJzdGF0dXMiOiI0MDEifQ==").is_empty());
    }
}
