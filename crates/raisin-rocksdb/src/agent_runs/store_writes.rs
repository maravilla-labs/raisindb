// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The staged writes of one run commit, applied as ONE node transaction.

use raisin_agent_runtime::record::AgentRunRecord;
use raisin_agent_runtime::store::StoreError;

use super::layout::{self as l, json};

/// Puts (path + string properties) and deletes (paths) of one commit.
#[derive(Default)]
pub(super) struct Writes {
    pub(super) puts: Vec<(String, Vec<(&'static str, String)>)>,
    pub(super) deletes: Vec<String>,
}

impl Writes {
    /// Put the node at `path`.
    pub(super) fn put(&mut self, path: String, props: Vec<(&'static str, String)>) {
        self.puts.push((path, props));
    }

    /// Delete the node at `path` (a no-op when absent).
    pub(super) fn delete(&mut self, path: String) {
        self.deletes.push(path);
    }

    /// Put the record node. Its flat properties are for humans and admin
    /// queries; the store reads only `record`.
    pub(super) fn record(&mut self, rec: &AgentRunRecord) -> Result<(), StoreError> {
        let mut props = vec![
            ("record", json(rec)?),
            ("status", rec.state.status().as_str().to_string()),
            ("subject_workspace", rec.subject.workspace.clone()),
            ("subject_path", rec.subject.path.clone()),
            ("version", rec.version.0.to_string()),
        ];
        if let Some(p) = &rec.parent_run_id {
            props.push(("parent_run_id", p.to_string()));
        }
        self.put(l::record(&rec.run_id)?, props);
        Ok(())
    }

    /// Move a worklist entry: add it when it becomes owed, drop it when paid.
    pub(super) fn worklist(&mut self, path: String, was: bool, now: bool) {
        match (was, now) {
            (false, true) => self.put(path, vec![]),
            (true, false) => self.delete(path),
            _ => {}
        }
    }
}

/// Whether a run owes a hand-back to whatever waits for it.
pub(super) fn handback_owed(rec: &AgentRunRecord) -> bool {
    rec.handback_owed()
}
