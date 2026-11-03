// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Work limits for graph searches that walk the per-node relation indexes.
//!
//! The ordinary scans stop at [`SCAN_TIME_LIMIT`] and [`SCAN_COUNT_CEILING`]
//! (see `scan_executors`). A graph search reads relations node by node, so it
//! counts what it reads the same way and watches the same clock: the
//! neighbourhood search (`NEIGHBORS(..., depth => n)`) and the seeded expansion
//! of a pinned variable-length `GRAPH_TABLE` pattern both carry a
//! [`GraphBudget`].
//!
//! The budget only decides WHEN to stop. What stopping means is the caller's
//! business: the neighbourhood search refuses the query with a generic error,
//! the seeded expansion hands the pattern back to the global route it replaced.

use std::time::{Duration, Instant};

use super::scan_executors::{SCAN_COUNT_CEILING, SCAN_TIME_LIMIT};

/// Which limit a search ran into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exhausted {
    /// More nodes visited than the search allows.
    Nodes,
    /// More relations read than the search allows.
    Relations,
    /// The wall-clock deadline passed.
    Time,
}

/// A deadline plus a ceiling on relations read, and the count so far.
#[derive(Debug, Clone)]
pub struct GraphBudget {
    deadline: Instant,
    max_relations: usize,
    relations_read: usize,
}

impl GraphBudget {
    /// The limits every scan has: [`SCAN_TIME_LIMIT`] from now and
    /// [`SCAN_COUNT_CEILING`] relations.
    pub fn standard() -> Self {
        Self::new(SCAN_TIME_LIMIT, SCAN_COUNT_CEILING)
    }

    /// `time_limit` from now, at most `max_relations` relations.
    pub fn new(time_limit: Duration, max_relations: usize) -> Self {
        Self::until(Instant::now() + time_limit, max_relations)
    }

    /// Stop at `deadline`, at most `max_relations` relations.
    pub fn until(deadline: Instant, max_relations: usize) -> Self {
        Self {
            deadline,
            max_relations,
            relations_read: 0,
        }
    }

    /// The same deadline with a different relation ceiling.
    pub fn with_max_relations(mut self, max_relations: usize) -> Self {
        self.max_relations = max_relations;
        self
    }

    /// The relation ceiling.
    pub fn max_relations(&self) -> usize {
        self.max_relations
    }

    /// Relations charged so far.
    pub fn relations_read(&self) -> usize {
        self.relations_read
    }

    /// Charge `relations` just read, then check the ceiling and the clock.
    pub fn charge(&mut self, relations: usize) -> Result<(), Exhausted> {
        self.relations_read = self.relations_read.saturating_add(relations);
        if self.relations_read > self.max_relations {
            return Err(Exhausted::Relations);
        }
        self.check_time()
    }

    /// Fail once the deadline has passed.
    pub fn check_time(&self) -> Result<(), Exhausted> {
        if Instant::now() >= self.deadline {
            Err(Exhausted::Time)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_is_inclusive_and_the_clock_is_checked_on_every_charge() {
        let mut b = GraphBudget::new(Duration::from_secs(60), 10);
        assert_eq!(b.charge(10), Ok(()));
        assert_eq!(b.charge(1), Err(Exhausted::Relations));

        let mut late = GraphBudget::until(Instant::now(), 1_000);
        assert_eq!(late.check_time(), Err(Exhausted::Time));
        assert_eq!(late.charge(0), Err(Exhausted::Time));
    }

    #[test]
    fn the_standard_budget_uses_the_scan_constants() {
        let b = GraphBudget::standard();
        assert_eq!(b.max_relations(), SCAN_COUNT_CEILING);
        assert!(b.check_time().is_ok());
    }
}
