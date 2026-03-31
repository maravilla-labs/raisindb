//! CSE configuration and context types

use super::arena::ExpressionArena;

/// Configuration for CSE optimization
///
/// Note: In addition to the `threshold` setting here, extraction is also gated
/// by a minimum cost threshold (`MIN_EXTRACTION_COST = 10` in `analyzer::cost`)
/// which prevents extracting cheap operations like `a + 1` (cost ~4) that aren't
/// worth the materialization overhead.
#[derive(Debug, Clone)]
pub struct CseConfig {
    /// Minimum number of occurrences required to extract an expression.
    ///
    /// Default: 2 (extract expressions that appear 2 or more times)
    ///
    /// Setting this higher (e.g., 3) reduces the number of intermediate projections
    /// but may miss optimization opportunities. Setting it to 1 would extract
    /// every expression, which is counterproductive.
    pub threshold: usize,
}

/// Context for CSE optimization pass
///
/// This context owns the ExpressionArena and is passed mutably through the
/// analysis and rewriting phases. This enables zero-copy expression sharing
/// via ExprId indices instead of cloning TypedExpr instances.
///
/// The context is Send (can be moved between threads) but optimization is
/// single-threaded per query (no synchronization needed).
#[derive(Debug)]
pub struct CseContext {
    /// Arena for storing expressions with zero-copy sharing
    pub(crate) arena: ExpressionArena,
    /// Configuration for CSE optimization
    pub(crate) config: CseConfig,
}

impl CseContext {
    /// Create a new CSE context with the given configuration
    pub fn new(config: CseConfig) -> Self {
        Self {
            arena: ExpressionArena::new(),
            config,
        }
    }

    /// Create a new CSE context with pre-allocated arena capacity
    pub fn with_capacity(config: CseConfig, capacity: usize) -> Self {
        Self {
            arena: ExpressionArena::with_capacity(capacity),
            config,
        }
    }
}

impl Default for CseConfig {
    fn default() -> Self {
        Self { threshold: 2 }
    }
}
