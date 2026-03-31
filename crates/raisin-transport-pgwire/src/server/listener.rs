// SPDX-License-Identifier: BSL-1.1

//! TCP listener and connection accept loop.

use crate::error::{PgWireTransportError, Result};
use pgwire::api::PgWireHandlerFactory;
use pgwire::tokio::process_socket;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use super::config::PgWireConfig;

/// PostgreSQL wire protocol server
///
/// This is the main server component that accepts TCP connections and processes
/// them using the PostgreSQL wire protocol.
pub struct PgWireServer<H>
where
    H: PgWireHandlerFactory + Send + Sync + 'static,
{
    /// Server configuration
    config: PgWireConfig,

    /// Handler factory for creating connection handlers
    pub(super) handler: Option<Arc<H>>,
}

impl<H> PgWireServer<H>
where
    H: PgWireHandlerFactory + Send + Sync + 'static,
{
    /// Create a new PostgreSQL wire protocol server
    pub fn new(config: PgWireConfig) -> Self {
        Self {
            config,
            handler: None,
        }
    }

    /// Set the handler factory for processing connections
    pub fn with_handler(mut self, handler: H) -> Self {
        self.handler = Some(Arc::new(handler));
        self
    }

    /// Run the server
    ///
    /// Starts the TCP listener and enters the main accept loop,
    /// processing incoming connections until the server is shut down.
    pub async fn run(&self) -> Result<()> {
        let handler = self
            .handler
            .as_ref()
            .ok_or_else(|| PgWireTransportError::internal("No handler configured for server"))?;

        let listener = TcpListener::bind(&self.config.bind_addr)
            .await
            .map_err(|e| {
                error!("Failed to bind to {}: {}", self.config.bind_addr, e);
                e
            })?;

        info!(
            "PostgreSQL wire protocol server listening on {}",
            self.config.bind_addr
        );
        info!(
            "Max connections: {}",
            if self.config.max_connections == 0 {
                "unlimited".to_string()
            } else {
                self.config.max_connections.to_string()
            }
        );

        let active_connections = Arc::new(tokio::sync::Semaphore::new(
            if self.config.max_connections == 0 {
                usize::MAX
            } else {
                self.config.max_connections
            },
        ));

        // Main accept loop
        loop {
            match listener.accept().await {
                Ok((socket, peer_addr)) => {
                    let permit = match active_connections.clone().try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            warn!(
                                "Connection limit reached ({}), rejecting connection from {}",
                                self.config.max_connections, peer_addr
                            );
                            continue;
                        }
                    };

                    info!("Accepted connection from {}", peer_addr);

                    let handler_ref = Arc::clone(handler);

                    tokio::spawn(async move {
                        info!("Processing connection from {}", peer_addr);

                        if let Err(e) = process_socket(socket, None, handler_ref).await {
                            error!("Error processing connection from {}: {}", peer_addr, e);
                        }

                        info!("Connection from {} closed", peer_addr);
                        drop(permit);
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                    continue;
                }
            }
        }
    }
}
