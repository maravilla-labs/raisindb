//! Method evaluation implementations
//!
//! This module contains the implementations for all built-in methods
//! that can be called on values in the expression language.

mod array;
mod path;
mod string;
mod universal;

pub use array::*;
pub use path::*;
pub use string::*;
pub use universal::*;
