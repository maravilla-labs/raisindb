//! Helper functions for hierarchy operations

/// Compute depth of a path
///
/// Used for constant folding and depth computation.
pub fn compute_depth(path: &str) -> i32 {
    path.split('/').filter(|s| !s.is_empty()).count() as i32
}

/// Compute parent path
///
/// Used for constant folding and parent computation.
pub fn compute_parent_path(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if parts.is_empty() {
        // Root path has no parent
        None
    } else if parts.len() == 1 {
        // Top-level item, parent is root
        Some("/".to_string())
    } else {
        // Build parent path from all but last segment
        Some(format!("/{}", parts[..parts.len() - 1].join("/")))
    }
}
