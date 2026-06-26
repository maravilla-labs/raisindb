// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Transport adapter over the dispatch engine.
//!
//! [`McpEndpoint`] binds a [`Dispatcher`] to a per-session [`McpIdentity`] and
//! turns raw JSON-RPC message strings into response strings. It is the bridge any
//! byte transport (stdio, a WebSocket frame handler, an HTTP request body) plugs
//! into: feed it a request line, write back the returned response line. The
//! reference [`run_stdio`](McpEndpoint::run_stdio) loop is provided for local
//! tooling and tests.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, warn};

use crate::dispatch::Dispatcher;
use crate::error::{McpError, Result};
use crate::identity::McpIdentity;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse};

/// A dispatch engine bound to one session identity, driven by a byte transport.
pub struct McpEndpoint {
    identity: McpIdentity,
    dispatcher: Dispatcher,
}

impl McpEndpoint {
    /// Bind a dispatcher to the identity that authenticated the session.
    pub fn new(identity: McpIdentity, dispatcher: Dispatcher) -> Self {
        Self {
            identity,
            dispatcher,
        }
    }

    /// The session identity.
    pub fn identity(&self) -> &McpIdentity {
        &self.identity
    }

    /// The underlying dispatch engine.
    pub fn dispatcher(&self) -> &Dispatcher {
        &self.dispatcher
    }

    /// Handle a single raw JSON-RPC message line.
    ///
    /// Returns the serialized response, or `None` for notifications (which the
    /// spec says receive no reply).
    pub async fn handle_message(&self, line: &str) -> Option<String> {
        let request: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(err) => {
                let err = McpError::parse(err.to_string());
                let response = JsonRpcResponse::failure(serde_json::Value::Null, &err);
                return serde_json::to_string(&response).ok();
            }
        };

        let id = request.id.clone();
        let outcome = self.dispatcher.handle(&self.identity, &request).await;

        match id {
            None => {
                if let Err(err) = outcome {
                    warn!(error = %err, method = %request.method, "notification handler failed");
                }
                None
            }
            Some(id) => {
                let response = match outcome {
                    Ok(result) => JsonRpcResponse::success(id, result),
                    Err(err) => {
                        debug!(error = %err, method = %request.method, "request failed");
                        JsonRpcResponse::failure(id, &err)
                    }
                };
                serde_json::to_string(&response).ok()
            }
        }
    }

    /// Run the newline-delimited JSON-RPC loop over stdio until EOF.
    ///
    /// Reference transport: each stdin line is one request, each response is one
    /// stdout line.
    pub async fn run_stdio(&self) -> Result<()> {
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin).lines();
        let mut stdout = tokio::io::stdout();

        while let Some(line) = reader
            .next_line()
            .await
            .map_err(|e| McpError::protocol(format!("stdin read error: {e}")))?
        {
            if line.trim().is_empty() {
                continue;
            }
            if let Some(response) = self.handle_message(&line).await {
                write_line(&mut stdout, &response).await?;
            }
        }
        Ok(())
    }
}

/// Write one response line and flush.
async fn write_line(stdout: &mut tokio::io::Stdout, response: &str) -> Result<()> {
    stdout
        .write_all(response.as_bytes())
        .await
        .map_err(|e| McpError::protocol(format!("stdout write error: {e}")))?;
    stdout
        .write_all(b"\n")
        .await
        .map_err(|e| McpError::protocol(format!("stdout write error: {e}")))?;
    stdout
        .flush()
        .await
        .map_err(|e| McpError::protocol(format!("stdout flush error: {e}")))?;
    Ok(())
}
