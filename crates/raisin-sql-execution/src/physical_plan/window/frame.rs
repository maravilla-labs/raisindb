//! Window frame bounds computation
//!
//! Determines which rows fall within the window frame for a given row.
//! Supports ROWS BETWEEN and RANGE BETWEEN frame specifications.

use raisin_sql::analyzer::FrameBound;

/// Determine window frame bounds for a row
///
/// Returns (start_idx, end_idx) where end_idx is exclusive (half-open range).
///
/// # Arguments
///
/// * `current_row` - Index of current row in partition
/// * `partition_size` - Total number of rows in partition
/// * `frame` - Optional frame specification
///
/// # Returns
///
/// (start_idx, end_idx) where range is [start_idx, end_idx)
pub(crate) fn determine_frame_bounds(
    current_row: usize,
    partition_size: usize,
    frame: &Option<raisin_sql::analyzer::WindowFrame>,
) -> (usize, usize) {
    // Default frame (when no frame specified):
    // - With ORDER BY: RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
    // - Without ORDER BY: entire partition
    // Since we are already sorted by ORDER BY, we treat no frame as entire partition
    let frame = match frame {
        Some(f) => f,
        None => {
            // Default: entire partition
            return (0, partition_size);
        }
    };

    // Compute start bound
    let start = match frame.start {
        FrameBound::UnboundedPreceding => 0,
        FrameBound::Preceding(n) => current_row.saturating_sub(n),
        FrameBound::CurrentRow => current_row,
        FrameBound::Following(n) => (current_row + n).min(partition_size),
        FrameBound::UnboundedFollowing => partition_size,
    };

    // Compute end bound
    let end = match frame.end {
        Some(FrameBound::UnboundedPreceding) => 0,
        Some(FrameBound::Preceding(n)) => current_row.saturating_sub(n),
        Some(FrameBound::CurrentRow) => current_row + 1, // Exclusive end
        Some(FrameBound::Following(n)) => (current_row + n + 1).min(partition_size),
        Some(FrameBound::UnboundedFollowing) => partition_size,
        None => current_row + 1, // Default: CURRENT ROW
    };

    (start, end)
}
