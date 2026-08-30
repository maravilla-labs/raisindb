// SPDX-License-Identifier: BSL-1.1

//! One consolidated integration-test binary for `raisin-indexer`.
//!
//! Add new test files here as `mod` lines — a file placed directly under
//! `tests/` would link its own binary (see CLAUDE.md, "Disk: watch target/").

mod index_node_durability;

mod language_analysis;
