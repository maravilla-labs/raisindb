//! Supporting types for storage operations

/// Statistics from job restoration after crash/restart
#[derive(Debug, Clone)]
pub struct RestoreStats {
    /// Number of jobs successfully restored to the registry
    pub restored: usize,
    /// Number of Running jobs that were reset to Scheduled
    pub reset_running: usize,
    /// Number of jobs that failed to restore (orphaned metadata)
    pub failed_to_restore: usize,
}
