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

//! Job type definitions for background task management

mod asset_processing;
mod category;
mod id;
mod index_operation;
mod info;
mod job_type;
mod job_type_display;
mod job_type_methods;
mod job_type_parse;
mod priority;
mod status;

pub use asset_processing::{AssetProcessingOptions, PdfExtractionStrategy};
pub use category::JobCategory;
pub use id::JobId;
pub use index_operation::{BatchIndexOperation, IndexOperation};
pub use info::{JobContext, JobHandle, JobInfo};
pub use job_type::JobType;
pub use priority::JobPriority;
pub use status::JobStatus;
