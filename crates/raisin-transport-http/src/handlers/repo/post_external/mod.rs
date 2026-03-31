// SPDX-License-Identifier: BSL-1.1

//! External storage upload handlers.
//!
//! Handles multipart uploads that store content in binary storage (not inline).
//! Supports both commit-mode (transaction with revision) and direct-mode (upsert).

mod commit;
mod direct;
mod handler;
#[cfg(feature = "storage-rocksdb")]
mod jobs;
mod resource;

pub(crate) use handler::handle_external_upload;
