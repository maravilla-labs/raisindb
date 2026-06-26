// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! HTTP surface for the OAuth 2.1 authorization server.
//!
//! These handlers expose the [`raisin_auth::authserver`] module over HTTP so an
//! interactive MCP client can discover the server, register itself, obtain an
//! authorization code, and exchange it for a resource-bound access token:
//!
//! - [`metadata`] — `/.well-known/oauth-authorization-server` (RFC 8414) and
//!   `/.well-known/oauth-protected-resource/...` (RFC 9728).
//! - [`register`] — `POST /register` (RFC 7591 Dynamic Client Registration).
//! - [`authorize`] — `GET`/`POST /authorize` (PKCE-gated code grant; the POST
//!   step authenticates the resource owner through the existing identity store).
//! - [`token`] — `POST /token` (authorization-code exchange + PKCE verification,
//!   minting a JWT via the resource server's existing signing machinery).
//!
//! All handlers require the `storage-rocksdb` feature, which is what brings the
//! `raisin-auth` authorization server, the identity store, and the JWT signer
//! into the build.

pub mod authorize;
mod consent;
pub mod helpers;
pub mod metadata;
pub mod register;
pub mod token;
