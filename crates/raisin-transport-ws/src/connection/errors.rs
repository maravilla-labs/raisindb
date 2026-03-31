// SPDX-License-Identifier: BSL-1.1

//! Error types for connection operations.

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("Channel not set")]
    ChannelNotSet,

    #[error("Channel closed")]
    ChannelClosed,
}

#[derive(Debug, thiserror::Error)]
pub enum AcquireError {
    #[error("Semaphore closed")]
    SemaphoreClosed,

    #[error("No permits available")]
    NoPermitsAvailable,
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    #[error("Transaction already active")]
    AlreadyActive,

    #[error("No active transaction")]
    NoActiveTransaction,
}
