use crate::analyzer::typed_expr::TypedExpr;
use std::fmt;

/// Index into the ExpressionArena (newtype for type safety)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExprId(u32);

impl ExprId {
    /// Create a new ExprId from a raw index
    #[inline]
    pub fn new(index: u32) -> Self {
        Self(index)
    }

    /// Get the raw index
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for ExprId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "expr_{}", self.0)
    }
}

/// Arena-based expression storage using Vector + Index pattern
///
/// This provides:
/// - Zero-copy expression sharing via lightweight ExprId indices
/// - Contiguous memory allocation for better cache locality
/// - Thread-safe (Send) but single-threaded usage (no locks needed)
/// - O(1) expression lookup by ID
///
/// Usage:
/// ```ignore
/// let mut arena = ExpressionArena::new();
/// let id1 = arena.add(expr1);
/// let id2 = arena.add(expr2);
/// let expr = arena.get(id1);
/// ```
#[derive(Debug)]
pub struct ExpressionArena {
    /// Storage for all expressions
    expressions: Vec<TypedExpr>,
}

impl ExpressionArena {
    /// Create a new empty arena
    pub fn new() -> Self {
        Self {
            expressions: Vec::new(),
        }
    }

    /// Create a new arena with pre-allocated capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            expressions: Vec::with_capacity(capacity),
        }
    }

    /// Add an expression to the arena and return its ID
    ///
    /// # Panics
    /// Panics if the arena contains more than u32::MAX expressions
    pub fn add(&mut self, expr: TypedExpr) -> ExprId {
        let index = self.expressions.len();
        assert!(
            index < u32::MAX as usize,
            "ExpressionArena overflow: cannot store more than {} expressions",
            u32::MAX
        );
        self.expressions.push(expr);
        ExprId::new(index as u32)
    }

    /// Get a reference to an expression by its ID
    ///
    /// # Panics
    /// Panics if the ID is invalid
    #[inline]
    pub fn get(&self, id: ExprId) -> &TypedExpr {
        &self.expressions[id.index()]
    }

    /// Get a mutable reference to an expression by its ID
    ///
    /// # Panics
    /// Panics if the ID is invalid
    #[inline]
    pub fn get_mut(&mut self, id: ExprId) -> &mut TypedExpr {
        &mut self.expressions[id.index()]
    }

    /// Try to get a reference to an expression by its ID
    ///
    /// Returns None if the ID is invalid
    #[inline]
    pub fn try_get(&self, id: ExprId) -> Option<&TypedExpr> {
        self.expressions.get(id.index())
    }

    /// Get the number of expressions in the arena
    #[inline]
    pub fn len(&self) -> usize {
        self.expressions.len()
    }

    /// Check if the arena is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.expressions.is_empty()
    }

    /// Clear all expressions from the arena
    pub fn clear(&mut self) {
        self.expressions.clear();
    }

    /// Iterator over all (ExprId, &TypedExpr) pairs
    pub fn iter(&self) -> impl Iterator<Item = (ExprId, &TypedExpr)> {
        self.expressions
            .iter()
            .enumerate()
            .map(|(idx, expr)| (ExprId::new(idx as u32), expr))
    }
}

impl Default for ExpressionArena {
    fn default() -> Self {
        Self::new()
    }
}

// ExpressionArena is Send (can be moved between threads) but not Sync
// (cannot be shared between threads without synchronization).
// This is correct for our use case: query optimization is single-threaded
// per query, but the optimizer itself can be moved between threads.
unsafe impl Send for ExpressionArena {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::typed_expr::Literal;

    #[test]
    fn test_arena_basic_operations() {
        let mut arena = ExpressionArena::new();

        let expr1 = TypedExpr::literal(Literal::Int(42));
        let expr2 = TypedExpr::literal(Literal::Text("hello".to_string()));

        let id1 = arena.add(expr1.clone());
        let id2 = arena.add(expr2.clone());

        assert_eq!(arena.len(), 2);
        assert!(!arena.is_empty());

        // Check retrieval
        assert!(matches!(
            arena.get(id1).expr,
            crate::analyzer::typed_expr::Expr::Literal(Literal::Int(42))
        ));

        assert!(matches!(
            arena.get(id2).expr,
            crate::analyzer::typed_expr::Expr::Literal(Literal::Text(ref s)) if s == "hello"
        ));
    }

    #[test]
    fn test_arena_with_capacity() {
        let arena = ExpressionArena::with_capacity(100);
        assert_eq!(arena.len(), 0);
        assert!(arena.is_empty());
        assert!(arena.expressions.capacity() >= 100);
    }

    #[test]
    fn test_expr_id_equality() {
        let id1 = ExprId::new(0);
        let id2 = ExprId::new(0);
        let id3 = ExprId::new(1);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_arena_iterator() {
        let mut arena = ExpressionArena::new();

        arena.add(TypedExpr::literal(Literal::Int(1)));
        arena.add(TypedExpr::literal(Literal::Int(2)));
        arena.add(TypedExpr::literal(Literal::Int(3)));

        let ids: Vec<_> = arena.iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], ExprId::new(0));
        assert_eq!(ids[1], ExprId::new(1));
        assert_eq!(ids[2], ExprId::new(2));
    }

    #[test]
    fn test_arena_clear() {
        let mut arena = ExpressionArena::new();
        arena.add(TypedExpr::literal(Literal::Int(42)));
        assert_eq!(arena.len(), 1);

        arena.clear();
        assert_eq!(arena.len(), 0);
        assert!(arena.is_empty());
    }

    #[test]
    fn test_try_get() {
        let mut arena = ExpressionArena::new();
        let id = arena.add(TypedExpr::literal(Literal::Int(42)));

        assert!(arena.try_get(id).is_some());
        assert!(arena.try_get(ExprId::new(999)).is_none());
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn test_invalid_id_panics() {
        let arena = ExpressionArena::new();
        let _ = arena.get(ExprId::new(0));
    }
}
