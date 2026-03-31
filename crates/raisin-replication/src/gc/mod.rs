//! Garbage Collection for Operation Log
//!
//! The GC system ensures bounded growth of the operation log while maintaining
//! replication correctness. It uses multiple strategies:
//!
//! 1. **Acknowledgment-based GC**: Delete operations acknowledged by all known peers
//! 2. **Time-based fail-safe**: Force delete after 30 days regardless of peer status
//! 3. **Size-based emergency GC**: Aggressive cleanup when log exceeds size limits
//!
//! ## Safety Guarantees
//!
//! - Never deletes unacknowledged operations from active peers
//! - Time-based fail-safe prevents permanent offline peers from blocking GC
//! - Emergency GC uses most aggressive policy when storage is critical
//! - Monotonic watermarks ensure we never re-delete operations

mod collector;
pub mod config;
#[cfg(test)]
mod tests;
pub mod watermarks;

pub use collector::GarbageCollector;
pub use config::{GcConfig, GcResult, GcStrategy};
pub use watermarks::PeerWatermarks;
