// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function flow types for multi-function trigger execution
//!
//! This module defines the data structures for executing multiple functions
//! in sequential and/or parallel patterns when a trigger fires.

mod definition;
mod enums;
mod results;

pub use definition::*;
pub use enums::*;
pub use results::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_flow() {
        let flow = FunctionFlow::new()
            .add_step(FlowStep::new(
                "step1",
                "Validate",
                "/functions/lib/validate",
            ))
            .add_step(
                FlowStep::new("step2", "Process", "/functions/lib/process").depends_on("step1"),
            );

        assert!(flow.validate().is_ok());
        let order = flow.execution_order().unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0].id, "step1");
        assert_eq!(order[1].id, "step2");
    }

    #[test]
    fn test_parallel_step() {
        let step = FlowStep::new("process", "Process", "/functions/lib/process_a")
            .add_function(FunctionRef::new("/functions/lib/process_b"))
            .parallel();

        assert!(step.parallel);
        assert_eq!(step.functions.len(), 2);
    }

    #[test]
    fn test_cyclic_dependency() {
        let flow = FunctionFlow::new()
            .add_step(FlowStep::new("step1", "Step 1", "/f1").depends_on("step2"))
            .add_step(FlowStep::new("step2", "Step 2", "/f2").depends_on("step1"));

        let result = flow.execution_order();
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_dependency() {
        let flow = FunctionFlow::new()
            .add_step(FlowStep::new("step1", "Step 1", "/f1").depends_on("nonexistent"));

        let result = flow.validate();
        assert!(matches!(
            result,
            Err(FlowValidationError::MissingDependency { .. })
        ));
    }
}
