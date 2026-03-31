//! Vector operations for similarity search and distance calculations

use raisin_error::Error;
use raisin_sql::analyzer::Literal;

/// Extract vector from a literal value
pub(super) fn extract_vector(literal: &Literal) -> Result<Vec<f32>, Error> {
    match literal {
        Literal::Vector(vec) => Ok(vec.clone()),
        _ => Err(Error::Validation(format!(
            "Expected vector, found: {:?}",
            literal
        ))),
    }
}

/// Calculate L2 (Euclidean) distance between two vectors
///
/// Formula: sqrt(sum((a[i] - b[i])^2))
///
/// Performance optimization: Uses fold instead of map+sum for better optimization
#[inline]
pub(super) fn l2_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::MAX; // Return maximum distance for dimension mismatch
    }

    let sum_squares = a.iter().zip(b.iter()).fold(0.0_f32, |acc, (x, y)| {
        let diff = x - y;
        acc + diff * diff
    });

    sum_squares.sqrt()
}

/// Calculate dot product of two vectors
///
/// Formula: sum(a[i] * b[i])
///
/// Performance optimization: Already uses iterator fusion
#[inline]
pub(super) fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0; // Return 0 for dimension mismatch
    }

    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}
