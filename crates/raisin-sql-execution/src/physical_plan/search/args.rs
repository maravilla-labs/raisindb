//! The argument grammar for `HYBRID_SEARCH`, `FULLTEXT_SEARCH` and `KNN`.
//!
//! ONE parser for all three. Three hand-maintained argument readers is the same
//! drift class as three hand-maintained column lists, and it had already
//! produced two live bugs in this file: `HYBRID_SEARCH` hard-coded
//! `language: "en"` while `FULLTEXT_SEARCH` took the language as an argument,
//! and `HYBRID_SEARCH('q', 'library')` silently became limit-10-cross-workspace
//! because a non-INT second positional fell back to the default.
//!
//! ```text
//! HYBRID_SEARCH  ( query [, limit] [, workspace] [, named ...] )
//! FULLTEXT_SEARCH( query ,  language            [, named ...] )
//! KNN            ( query [, limit]              [, named ...] )
//! ```
//!
//! Named arguments use `=>` only. `sqlparser` 0.59 defaults
//! `supports_named_fn_args_with_rarrow_operator` to true and the assignment form
//! (`:=`) to false, so there is no second spelling to support.
//!
//! `$1` and friends never reach here: parameters are substituted into the SQL
//! text before parsing (`raisin_sql::substitute_params`, used by both the HTTP
//! and the pgwire extended-query paths), so an agent building a workspace list
//! in its host language binds it and this sees a plain literal.

use raisin_sql::analyzer::{Expr, Literal, TableFunctionArg, TypedExpr};

use crate::physical_plan::executor::ExecutionError;

use super::scope::{parse_workspace_scope, WorkspaceScopeSpec};
use super::vector_of::VectorOfRef;

/// Which of the three surfaces is being called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFunction {
    Hybrid,
    Fulltext,
    Knn,
}

impl SearchFunction {
    pub fn name(self) -> &'static str {
        match self {
            SearchFunction::Hybrid => "HYBRID_SEARCH",
            SearchFunction::Fulltext => "FULLTEXT_SEARCH",
            SearchFunction::Knn => "KNN",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("HYBRID_SEARCH") {
            Some(SearchFunction::Hybrid)
        } else if name.eq_ignore_ascii_case("FULLTEXT_SEARCH") {
            Some(SearchFunction::Fulltext)
        } else if name.eq_ignore_ascii_case("KNN") {
            Some(SearchFunction::Knn)
        } else {
            None
        }
    }

    fn default_limit(self) -> usize {
        match self {
            SearchFunction::Hybrid | SearchFunction::Knn => 10,
            SearchFunction::Fulltext => 100,
        }
    }

    /// The named arguments this function accepts, in the order the error lists
    /// them. `KNN` rejects the weights and the language: it has no full-text leg
    /// to weight and no analyzer to choose.
    fn valid_named(self) -> &'static [&'static str] {
        match self {
            SearchFunction::Hybrid => &[
                "workspaces",
                "limit",
                "language",
                "vector_weight",
                "fulltext_weight",
                "max_distance",
                "kind",
            ],
            // No `kind`: FULLTEXT_SEARCH has no vector leg, so there is no
            // embedding space to select. Accepting it and ignoring it is how
            // the third positional came to mean nothing on this function.
            SearchFunction::Fulltext => &["workspaces", "limit", "language"],
            SearchFunction::Knn => &["workspaces", "limit", "max_distance", "kind"],
        }
    }
}

/// Which embedding space(s) the vector leg reads.
///
/// `kind` is not a new concept invented for the query surface: it is segment 6
/// of the `cf::EMBEDDINGS` key and the last character of a `PartitionId` token,
/// and it has been there since before anything queried it. This enum selects
/// over that, and resolution routes to real partitions in
/// [`super::legs::resolve_vector_partitions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbeddingKindFilter {
    /// Text vectors only. THE DEFAULT, and deliberately not `All`.
    ///
    /// Defaulting to `All` would mean that on the day an image tower first
    /// writes a vector, every existing query silently starts fusing a second
    /// corpus in -- new rows in every result set, no error, no change to any
    /// query text, and a `LIMIT 10` that now spends slots on pictures. That is
    /// this codebase's named #1 bug class delivered by a default. Breadth is
    /// opt-in and says so in the query, exactly as `workspaces => 'ALL
    /// READABLE'` is.
    #[default]
    Text,
    /// Image vectors only.
    Image,
    /// Every configured space, rank-fused. One leg per partition.
    All,
}

impl EmbeddingKindFilter {
    /// Parse the `kind` value. Public because the HTTP and MCP surfaces take
    /// the same three words and must not grow a second spelling of them.
    pub fn parse(raw: &str, function: &str) -> Result<Self, ExecutionError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "image" => Ok(Self::Image),
            "all" => Ok(Self::All),
            other => Err(ExecutionError::Validation(format!(
                "{function}: kind must be 'text', 'image' or 'all', got '{other}'."
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::All => "all",
        }
    }

    /// Does this selector admit a partition whose token ends in `kind_char`?
    pub fn admits(self, kind_char: char) -> bool {
        match self {
            Self::All => true,
            Self::Text => kind_char == 'T',
            Self::Image => kind_char == 'I',
        }
    }
}

/// What the caller wants matched against.
#[derive(Debug, Clone)]
pub enum QueryInput {
    /// Text to embed and/or analyse.
    Text(String),
    /// A vector supplied directly (`KNN` only). No embedding call is made.
    ///
    /// Reached three ways, all of which land here rather than in three parsers:
    /// a `Literal::Vector`, an `ARRAY[...]` (what the functions runtime renders
    /// a bound array into), and the pgvector text form `'[0.1,0.2]'` (what the
    /// HTTP and pgwire substituters render one into).
    Vector(Vec<f32>),
    /// `VECTOR_OF(...)` — a node's own STORED vector, resolved against
    /// `cf::EMBEDDINGS` at execution time (`KNN` only).
    ///
    /// Unresolved here on purpose: parsing is synchronous and has no storage,
    /// and the vector is per-PARTITION anyway (`kind => 'all'` asks two
    /// embedding spaces for the same node's two different vectors).
    StoredVector(VectorOfRef),
}

impl QueryInput {
    /// The text, when there is any. `None` for anything vector-shaped, which
    /// has no lexical surface and therefore no full-text leg.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            QueryInput::Text(t) => Some(t),
            QueryInput::Vector(_) | QueryInput::StoredVector(_) => None,
        }
    }

    /// True when the vector leg needs no embedding provider: the caller either
    /// supplied the vector or named a node whose vector is already stored.
    ///
    /// ONE predicate, because the provider-resolution guard and the
    /// does-the-vector-leg-run guard used to test `matches!(.., Vector(_))`
    /// separately, and a third variant that satisfied one but not the other
    /// would resolve a provider it never uses — or, worse, refuse the query on
    /// a tenant that has no embedder and does not need one.
    pub fn needs_no_provider(&self) -> bool {
        matches!(self, QueryInput::Vector(_) | QueryInput::StoredVector(_))
    }
}

/// A fully validated call.
#[derive(Debug, Clone)]
pub struct SearchArgs {
    pub function: SearchFunction,
    pub query: QueryInput,
    pub limit: usize,
    /// Exactly what the caller wrote, for EXPLAIN and the operator log.
    pub scope_spec_raw: String,
    pub scope_spec: WorkspaceScopeSpec,
    pub language: String,
    /// `0.0` skips the full-text leg entirely: no Tantivy query at all.
    pub fulltext_weight: f64,
    /// `0.0` skips the vector leg entirely -- including embedding-provider
    /// RESOLUTION, so a tenant with no embedder can still run a deliberately
    /// full-text-only hybrid query.
    pub vector_weight: f64,
    pub max_distance: f32,
    /// Which embedding space(s) the vector leg reads.
    pub kind: EmbeddingKindFilter,
}

impl SearchArgs {
    pub fn runs_fulltext(&self) -> bool {
        self.fulltext_weight > 0.0 && self.query.as_text().is_some()
    }

    pub fn runs_vector(&self) -> bool {
        self.vector_weight > 0.0
    }
}

impl SearchArgs {
    /// Build a call from a NON-SQL surface (the HTTP hybrid-search endpoint, the
    /// MCP `search_nodes` tool) without going through SQL text.
    ///
    /// It exists so those surfaces stop being second and third implementations.
    /// Both used to run their own leg dispatch and their own fusion, and neither
    /// module contained the word `auth`: the HTTP handler keyed its RRF map on
    /// `node_id` alone (the very bug the SQL side fixed by keying on
    /// `(workspace_id, node_id)`), and the MCP provider took an `McpIdentity` it
    /// named `_identity`. Routing them here gives them the shared scope
    /// resolver, the shared RLS pass and the shared emit loop for free.
    ///
    /// The scope string uses the same grammar as `workspaces => ...`, so an API
    /// caller and a SQL caller cannot mean different things by the same words.
    pub fn from_api(
        function: SearchFunction,
        query: QueryInput,
        limit: usize,
        workspaces: &str,
        language: Option<&str>,
        default_language: &str,
        fulltext_weight: f64,
        vector_weight: f64,
        max_distance: Option<f32>,
        kind: EmbeddingKindFilter,
    ) -> Result<Self, ExecutionError> {
        let name = function.name();
        if limit == 0 || limit > 1000 {
            return Err(ExecutionError::Validation(format!(
                "{name}: limit must be between 1 and 1000, got {limit}."
            )));
        }
        let scope_spec = parse_workspace_scope(workspaces)?;
        let language = language.unwrap_or(default_language).to_string();
        if function != SearchFunction::Knn {
            validate_language(&language, name)?;
        }
        if fulltext_weight == 0.0 && vector_weight == 0.0 {
            return Err(ExecutionError::Validation(format!(
                "{name}: fulltext_weight and vector_weight are both 0, so neither \
                 half of the search would run."
            )));
        }
        Ok(Self {
            function,
            query,
            limit,
            scope_spec_raw: workspaces.to_string(),
            scope_spec,
            language,
            fulltext_weight,
            vector_weight,
            max_distance: max_distance.unwrap_or(0.6),
            kind,
        })
    }
}

/// Parse and validate one call.
pub fn parse_search_args(
    function: SearchFunction,
    args: &[TableFunctionArg],
    default_language: &str,
) -> Result<SearchArgs, ExecutionError> {
    let name = function.name();

    // Positionals must precede named arguments. Otherwise "the third argument"
    // stops being a stable idea and the workspace positional becomes ambiguous.
    let mut seen_named = false;
    for arg in args {
        match &arg.name {
            Some(_) => seen_named = true,
            None if seen_named => {
                return Err(ExecutionError::Validation(format!(
                    "{name}: positional arguments must come before named ones."
                )))
            }
            None => {}
        }
    }

    let positional: Vec<&TypedExpr> = args
        .iter()
        .filter(|a| a.name.is_none())
        .map(|a| &a.value)
        .collect();

    let mut named: Vec<(&str, &TypedExpr)> = Vec::new();
    for arg in args.iter().filter(|a| a.name.is_some()) {
        let key = arg.name.as_deref().unwrap();
        let lower = key.to_ascii_lowercase();
        if !function.valid_named().contains(&lower.as_str()) {
            return Err(ExecutionError::Validation(format!(
                "unknown argument '{key}' for {name}. Valid: {}.",
                function.valid_named().join(", ")
            )));
        }
        if named.iter().any(|(k, _)| *k == lower) {
            return Err(ExecutionError::Validation(format!(
                "{name}: argument '{lower}' was given twice."
            )));
        }
        // Leak a 'static-lifetimed lowercase key by matching against the known
        // list rather than allocating: the list IS the set of valid names.
        let canonical = function
            .valid_named()
            .iter()
            .find(|v| **v == lower)
            .copied()
            .expect("checked above");
        named.push((canonical, &arg.value));
    }

    let get_named = |key: &str| named.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);

    // ---- positional #1: the query -----------------------------------------
    let query_expr = positional.first().copied().ok_or_else(|| {
        ExecutionError::Validation(format!("{name} requires a query as its first argument."))
    })?;
    let query = parse_query_input(function, query_expr)?;

    // ---- positional #2 ------------------------------------------------------
    let mut limit_from_positional: Option<usize> = None;
    let mut language_from_positional: Option<String> = None;
    match function {
        SearchFunction::Fulltext => {
            // Still REQUIRED and still positional #2: existing callers wrote it
            // there and it is not being moved.
            let expr = positional.get(1).copied().ok_or_else(|| {
                ExecutionError::Validation(format!(
                    "{name} requires two positional arguments (query, language). \
                     The language is an ISO 639-1 code such as 'en'."
                ))
            })?;
            language_from_positional = Some(expect_text(expr, name, "language")?);
        }
        SearchFunction::Hybrid | SearchFunction::Knn => {
            if let Some(expr) = positional.get(1).copied() {
                // A non-INT here used to fall back to 10 with no complaint, so
                // HYBRID_SEARCH('q', 'library') meant limit 10 across every
                // workspace and looked like it had been scoped.
                limit_from_positional = Some(expect_int(expr, name, "limit")?);
            }
        }
    }

    // ---- positional #3 ------------------------------------------------------
    let mut scope_from_positional: Option<String> = None;
    if let Some(expr) = positional.get(2).copied() {
        match function {
            SearchFunction::Hybrid => {
                // Kept forever. This positional ALREADY means "the workspace",
                // so it keeps meaning it; the migration note says so.
                scope_from_positional = Some(expect_text(expr, name, "workspace")?);
            }
            SearchFunction::Fulltext | SearchFunction::Knn => {
                // Ignored today -- so giving it a meaning silently is exactly the
                // drift being banned. The asymmetry with HYBRID_SEARCH is
                // deliberate; do not "make it consistent".
                return Err(ExecutionError::Validation(format!(
                    "{name} takes no third positional argument. To scope the \
                     search write workspaces => '<workspace>'."
                )));
            }
        }
    }

    if positional.len() > 3 {
        return Err(ExecutionError::Validation(format!(
            "{name} takes at most 3 positional arguments; {} were given. \
             Everything else is a named argument (name => value).",
            positional.len()
        )));
    }

    // ---- workspaces ---------------------------------------------------------
    let scope_raw = match (scope_from_positional, get_named("workspaces")) {
        (Some(_), Some(_)) => {
            return Err(ExecutionError::Validation(format!(
                "{name}: the workspace was given both as the third positional \
                 argument and as workspaces => ...; keep one."
            )))
        }
        (Some(p), None) => p,
        (None, Some(expr)) => expect_text(expr, name, "workspaces")?,
        (None, None) => {
            return Err(ExecutionError::Validation(format!(
                "{name} requires an explicit workspace scope. Add \
                 workspaces => '<workspace>' to search one, 'a, b, c' for \
                 several, 'content-*' for a family, or \
                 workspaces => 'ALL READABLE' for every workspace you may read \
                 (which is what this call used to do)."
            )))
        }
    };
    let scope_spec = parse_workspace_scope(&scope_raw)?;

    // ---- limit --------------------------------------------------------------
    let limit = match (limit_from_positional, get_named("limit")) {
        (Some(_), Some(_)) => {
            return Err(ExecutionError::Validation(format!(
                "{name}: the limit was given both positionally and as \
                 limit => ...; keep one."
            )))
        }
        (Some(p), None) => p,
        (None, Some(expr)) => expect_int(expr, name, "limit")?,
        (None, None) => function.default_limit(),
    };
    if limit == 0 || limit > 1000 {
        return Err(ExecutionError::Validation(format!(
            "{name}: limit must be between 1 and 1000, got {limit}."
        )));
    }

    // ---- language -----------------------------------------------------------
    let language = match (language_from_positional, get_named("language")) {
        (Some(_), Some(_)) => {
            return Err(ExecutionError::Validation(format!(
                "{name}: the language was given both positionally and as \
                 language => ...; keep one."
            )))
        }
        (Some(p), None) => p,
        (None, Some(expr)) => expect_text(expr, name, "language")?,
        // Never a hard-coded "en", and never inferred from the query text --
        // inference would be a silent behaviour change dressed as a feature.
        (None, None) => default_language.to_string(),
    };
    if function != SearchFunction::Knn {
        validate_language(&language, name)?;
    }

    // ---- weights ------------------------------------------------------------
    let (fulltext_weight, vector_weight) = match function {
        SearchFunction::Knn => (0.0, 1.0),
        SearchFunction::Fulltext => (1.0, 0.0),
        SearchFunction::Hybrid => {
            let ft = match get_named("fulltext_weight") {
                Some(expr) => expect_double(expr, name, "fulltext_weight")?,
                None => 1.0,
            };
            let vec = match get_named("vector_weight") {
                Some(expr) => expect_double(expr, name, "vector_weight")?,
                None => 1.0,
            };
            for (label, w) in [("fulltext_weight", ft), ("vector_weight", vec)] {
                if !(w.is_finite() && w >= 0.0) {
                    return Err(ExecutionError::Validation(format!(
                        "{name}: {label} must be a finite number >= 0.0, got {w}."
                    )));
                }
            }
            if ft == 0.0 && vec == 0.0 {
                return Err(ExecutionError::Validation(format!(
                    "{name}: fulltext_weight and vector_weight are both 0, so \
                     neither half of the search would run. Set at least one \
                     above 0."
                )));
            }
            (ft, vec)
        }
    };

    // ---- max_distance -------------------------------------------------------
    let max_distance = match get_named("max_distance") {
        Some(expr) => {
            let d = expect_double(expr, name, "max_distance")?;
            if !(d.is_finite() && d > 0.0 && d <= 2.0) {
                return Err(ExecutionError::Validation(format!(
                    "{name}: max_distance must be in (0.0, 2.0], got {d}. \
                     Cosine distance on normalised vectors ranges 0.0 (identical) \
                     to 2.0 (opposite)."
                )));
            }
            d as f32
        }
        // Unchanged from the engine default, so nothing moves unless asked.
        None => 0.6,
    };

    // ---- kind ---------------------------------------------------------------
    let kind = match get_named("kind") {
        Some(expr) => EmbeddingKindFilter::parse(&expect_text(expr, name, "kind")?, name)?,
        None => EmbeddingKindFilter::default(),
    };

    Ok(SearchArgs {
        function,
        query,
        limit,
        scope_spec_raw: scope_raw,
        scope_spec,
        language,
        fulltext_weight,
        vector_weight,
        max_distance,
        kind,
    })
}

/// `KNN` argument 1 accepts five forms, in this order.
///
/// ```text
/// KNN('some text')                     -- embedded with the tenant's provider
/// KNN(EMBEDDING('some text'))          -- identical; the wrapper is unwrapped
/// KNN(ARRAY[0.1, 0.2, ...])            -- a vector literal
/// KNN('[0.1, 0.2, ...]')               -- the pgvector text form; see below
/// KNN(VECTOR_OF('assets:/cat.jpg'))    -- a node's own stored vector
/// ```
///
/// The fourth is what makes BINDING a vector work. `$1` never reaches this
/// function — parameters are substituted into the SQL text before it is parsed
/// — and the HTTP/pgwire substituter renders a bound JSON array as the quoted
/// string `'[0.1,0.2]'`. Without the check below that string was EMBEDDED as
/// text: a plausible ranking over a nonsense query vector, with nothing logged.
fn parse_query_input(
    function: SearchFunction,
    expr: &TypedExpr,
) -> Result<QueryInput, ExecutionError> {
    let name = function.name();
    match &expr.expr {
        Expr::Literal(Literal::Text(t)) | Expr::Literal(Literal::Path(t)) => {
            // A bound vector arrives here as text. `parse_vector_text` is
            // strict — brackets, non-empty, every element a finite number — so
            // prose is untouched. See `raisin_sql::analyzer::vector_literal`
            // for why the ambiguity is taken in this direction.
            if function == SearchFunction::Knn {
                if let Some(v) = raisin_sql::analyzer::parse_vector_text(t) {
                    tracing::debug!(
                        dims = v.len(),
                        "KNN argument 1 recognised as a bound vector in pgvector text form"
                    );
                    return Ok(QueryInput::Vector(v));
                }
            }
            Ok(QueryInput::Text(t.clone()))
        }
        Expr::Literal(Literal::Vector(v)) if function == SearchFunction::Knn => {
            Ok(QueryInput::Vector(v.clone()))
        }
        // VECTOR_OF('workspace:/path' [, chunk]) — read, not recomputed. Only
        // KNN, for the same reason a literal vector is only KNN: it has no
        // lexical surface, so a HYBRID_SEARCH built on it would silently be a
        // vector-only search reported as hybrid.
        Expr::Function {
            name: fname, args, ..
        } if fname.eq_ignore_ascii_case("VECTOR_OF") => {
            if function != SearchFunction::Knn {
                return Err(ExecutionError::Validation(format!(
                    "{name}: VECTOR_OF(...) is a vector, and {name} needs text for \
                     its full-text half. Use KNN(VECTOR_OF(...)) for \
                     similar-to-this-node search."
                )));
            }
            let reference = match args.first().map(|a| &a.expr) {
                Some(Expr::Literal(Literal::Text(t))) | Some(Expr::Literal(Literal::Path(t))) => {
                    t.clone()
                }
                _ => {
                    return Err(ExecutionError::Validation(format!(
                        "{name}: VECTOR_OF(...) takes a node reference as a text \
                         literal, e.g. VECTOR_OF('assets:/photos/cat.jpg')."
                    )))
                }
            };
            let chunk = match args.get(1) {
                None => None,
                Some(arg) => Some(expect_int(arg, name, "VECTOR_OF chunk index")?),
            };
            if args.len() > 2 {
                return Err(ExecutionError::Validation(format!(
                    "{name}: VECTOR_OF takes at most two arguments (node reference, \
                     chunk index), got {}.",
                    args.len()
                )));
            }
            Ok(QueryInput::StoredVector(VectorOfRef::parse(
                &reference, chunk,
            )?))
        }
        // EMBEDDING('...') is UNWRAPPED to its inner text literal and embedded
        // exactly like form 1. Identical result, and no scalar-expression
        // evaluator is needed at bind time -- which is what keeps the worked
        // example shipped in the keyword help working.
        Expr::Function {
            name: fname, args, ..
        } if function == SearchFunction::Knn && fname.eq_ignore_ascii_case("EMBEDDING") => {
            match args.first().map(|a| &a.expr) {
                Some(Expr::Literal(Literal::Text(t))) => Ok(QueryInput::Text(t.clone())),
                _ => Err(ExecutionError::Validation(format!(
                    "{name}: EMBEDDING(...) must wrap a text literal."
                ))),
            }
        }
        Expr::Literal(Literal::Parameter(p)) => Err(ExecutionError::Validation(format!(
            "{name}: parameter {p} was not bound. Parameters are substituted \
             into the statement before it is analysed."
        ))),
        _ if function == SearchFunction::Knn => Err(ExecutionError::Validation(format!(
            "{name} argument 1 must be a text literal, EMBEDDING('<text>'), or a \
             vector literal."
        ))),
        _ => Err(ExecutionError::Validation(format!(
            "{name} argument 1 must be a text literal."
        ))),
    }
}

fn expect_text(expr: &TypedExpr, function: &str, arg: &str) -> Result<String, ExecutionError> {
    match &expr.expr {
        Expr::Literal(Literal::Text(v)) | Expr::Literal(Literal::Path(v)) => Ok(v.clone()),
        Expr::Literal(Literal::Parameter(p)) => Err(ExecutionError::Validation(format!(
            "{function}: parameter {p} for '{arg}' was not bound."
        ))),
        other => Err(ExecutionError::Validation(format!(
            "{function}: '{arg}' must be a text literal, got {other:?}."
        ))),
    }
}

fn expect_int(expr: &TypedExpr, function: &str, arg: &str) -> Result<usize, ExecutionError> {
    let value = match &expr.expr {
        Expr::Literal(Literal::Int(n)) => *n as i64,
        Expr::Literal(Literal::BigInt(n)) => *n,
        Expr::Literal(Literal::Parameter(p)) => {
            return Err(ExecutionError::Validation(format!(
                "{function}: parameter {p} for '{arg}' was not bound."
            )))
        }
        other => {
            return Err(ExecutionError::Validation(format!(
                "{function}: '{arg}' must be an integer literal, got {other:?}. \
                 (It used to fall back to the default here, which is how \
                 {function}('q', 'library') became an unscoped search.)"
            )))
        }
    };
    if value < 0 {
        return Err(ExecutionError::Validation(format!(
            "{function}: '{arg}' must not be negative, got {value}."
        )));
    }
    Ok(value as usize)
}

fn expect_double(expr: &TypedExpr, function: &str, arg: &str) -> Result<f64, ExecutionError> {
    match &expr.expr {
        Expr::Literal(Literal::Double(v)) => Ok(*v),
        Expr::Literal(Literal::Int(v)) => Ok(*v as f64),
        Expr::Literal(Literal::BigInt(v)) => Ok(*v as f64),
        Expr::Literal(Literal::Parameter(p)) => Err(ExecutionError::Validation(format!(
            "{function}: parameter {p} for '{arg}' was not bound."
        ))),
        other => Err(ExecutionError::Validation(format!(
            "{function}: '{arg}' must be a number, got {other:?}."
        ))),
    }
}

/// The full-text index stores ISO 639-1 codes and the query builds an EXACT
/// `TermQuery` on that field, so `'english'` matches zero documents forever --
/// which is what the previously shipped help example said to write.
fn validate_language(language: &str, function: &str) -> Result<(), ExecutionError> {
    let ok = language.len() == 2 && language.bytes().all(|b| b.is_ascii_lowercase());
    if ok {
        return Ok(());
    }
    Err(ExecutionError::Validation(format!(
        "{function}: language must be an ISO 639-1 code. Use 'en', not \
         '{language}'; the index stores two-letter codes, so anything else \
         matches no documents."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_sql::analyzer::types::DataType;

    fn text(v: &str) -> TypedExpr {
        TypedExpr::new(Expr::Literal(Literal::Text(v.to_string())), DataType::Text)
    }
    fn int(v: i64) -> TypedExpr {
        TypedExpr::new(Expr::Literal(Literal::BigInt(v)), DataType::BigInt)
    }
    fn dbl(v: f64) -> TypedExpr {
        TypedExpr::new(Expr::Literal(Literal::Double(v)), DataType::Double)
    }

    fn hybrid(args: Vec<TableFunctionArg>) -> Result<SearchArgs, ExecutionError> {
        parse_search_args(SearchFunction::Hybrid, &args, "en")
    }

    /// The whole point: the name has to survive analysis and be READ.
    #[test]
    fn named_workspaces_is_not_a_positional() {
        let a = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::positional(int(10)),
            TableFunctionArg::named("workspaces", text("library")),
        ])
        .unwrap();
        assert_eq!(a.limit, 10);
        assert_eq!(
            a.scope_spec,
            WorkspaceScopeSpec::Exact(vec!["library".into()])
        );
    }

    /// The third positional keeps its meaning, forever.
    #[test]
    fn third_positional_equals_the_named_form() {
        let positional = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::positional(int(10)),
            TableFunctionArg::positional(text("library")),
        ])
        .unwrap();
        let named = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::positional(int(10)),
            TableFunctionArg::named("workspaces", text("library")),
        ])
        .unwrap();
        assert_eq!(positional.scope_spec, named.scope_spec);
        assert_eq!(positional.limit, named.limit);
    }

    #[test]
    fn the_two_argument_form_names_every_migration() {
        let err = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::positional(int(10)),
        ])
        .unwrap_err()
        .to_string();
        for fragment in ["workspaces =>", "a, b, c", "content-*", "ALL READABLE"] {
            assert!(err.contains(fragment), "error omits {fragment}: {err}");
        }
    }

    /// It used to silently mean limit 10 across every workspace.
    #[test]
    fn a_non_integer_limit_is_an_error_not_a_default() {
        assert!(hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::positional(text("library")),
        ])
        .is_err());
    }

    #[test]
    fn unknown_named_argument_lists_the_valid_ones() {
        let err = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::named("workspaces", text("library")),
            TableFunctionArg::named("k", int(60)),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("unknown argument 'k'"), "{err}");
        assert!(err.contains("max_distance"), "{err}");
    }

    #[test]
    fn a_value_given_twice_names_both_spellings() {
        let err = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::positional(int(10)),
            TableFunctionArg::positional(text("library")),
            TableFunctionArg::named("workspaces", text("handbook")),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("third positional"), "{err}");
    }

    #[test]
    fn both_weights_zero_is_an_error() {
        assert!(hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::named("workspaces", text("library")),
            TableFunctionArg::named("vector_weight", dbl(0.0)),
            TableFunctionArg::named("fulltext_weight", dbl(0.0)),
        ])
        .is_err());
    }

    /// Weight 0 must SKIP the leg, which is what lets a tenant with no embedder
    /// run a deliberately full-text-only hybrid query.
    #[test]
    fn vector_weight_zero_skips_the_vector_leg() {
        let a = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::named("workspaces", text("library")),
            TableFunctionArg::named("vector_weight", dbl(0.0)),
        ])
        .unwrap();
        assert!(!a.runs_vector());
        assert!(a.runs_fulltext());
    }

    /// Defaulting to `Text` is what keeps the day an image tower first writes a
    /// vector from silently changing every existing query's result set.
    #[test]
    fn kind_defaults_to_text_not_all() {
        let a = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::named("workspaces", text("library")),
        ])
        .unwrap();
        assert_eq!(a.kind, EmbeddingKindFilter::Text);
        assert!(a.kind.admits('T'));
        assert!(!a.kind.admits('I'));
    }

    #[test]
    fn kind_selects_the_three_spaces() {
        for (word, expected, admits_text, admits_image) in [
            ("text", EmbeddingKindFilter::Text, true, false),
            ("image", EmbeddingKindFilter::Image, false, true),
            ("all", EmbeddingKindFilter::All, true, true),
            // Case-insensitive, like every other word-valued argument here.
            ("ALL", EmbeddingKindFilter::All, true, true),
        ] {
            let a = hybrid(vec![
                TableFunctionArg::positional(text("q")),
                TableFunctionArg::named("workspaces", text("library")),
                TableFunctionArg::named("kind", text(word)),
            ])
            .unwrap_or_else(|e| panic!("kind => '{word}' must parse: {e}"));
            assert_eq!(a.kind, expected, "kind => '{word}'");
            assert_eq!(a.kind.admits('T'), admits_text);
            assert_eq!(a.kind.admits('I'), admits_image);
        }
    }

    #[test]
    fn an_unknown_kind_names_the_three_valid_words() {
        let err = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::named("workspaces", text("library")),
            TableFunctionArg::named("kind", text("pictures")),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("'text'"), "{err}");
        assert!(err.contains("'image'"), "{err}");
        assert!(err.contains("'all'"), "{err}");
    }

    /// `KNN` takes `kind` (it is all vector leg); `FULLTEXT_SEARCH` refuses it,
    /// because it has no vector leg and accepting-then-ignoring is exactly how
    /// the third positional came to mean nothing.
    #[test]
    fn knn_takes_kind_and_fulltext_refuses_it() {
        let knn = parse_search_args(
            SearchFunction::Knn,
            &[
                TableFunctionArg::positional(text("q")),
                TableFunctionArg::named("workspaces", text("library")),
                TableFunctionArg::named("kind", text("image")),
            ],
            "en",
        )
        .expect("KNN accepts kind");
        assert_eq!(knn.kind, EmbeddingKindFilter::Image);

        let err = parse_search_args(
            SearchFunction::Fulltext,
            &[
                TableFunctionArg::positional(text("q")),
                TableFunctionArg::positional(text("en")),
                TableFunctionArg::named("workspaces", text("library")),
                TableFunctionArg::named("kind", text("text")),
            ],
            "en",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("unknown argument 'kind'"), "{err}");
    }

    #[test]
    fn defaults_reproduce_todays_arithmetic_exactly() {
        let a = hybrid(vec![
            TableFunctionArg::positional(text("q")),
            TableFunctionArg::named("workspaces", text("library")),
        ])
        .unwrap();
        assert_eq!(a.fulltext_weight, 1.0);
        assert_eq!(a.vector_weight, 1.0);
        assert_eq!(a.max_distance, 0.6);
        assert_eq!(a.limit, 10);
    }

    #[test]
    fn language_must_be_iso_639_1() {
        let err = parse_search_args(
            SearchFunction::Fulltext,
            &[
                TableFunctionArg::positional(text("q")),
                TableFunctionArg::positional(text("english")),
                TableFunctionArg::named("workspaces", text("library")),
            ],
            "en",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("'en'"), "{err}");
    }

    /// Ignored today, so it must become loud rather than quietly meaningful.
    #[test]
    fn fulltext_rejects_a_third_positional_naming_the_named_form() {
        let err = parse_search_args(
            SearchFunction::Fulltext,
            &[
                TableFunctionArg::positional(text("q")),
                TableFunctionArg::positional(text("en")),
                TableFunctionArg::positional(text("library")),
            ],
            "en",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("workspaces =>"), "{err}");
    }

    #[test]
    fn knn_rejects_weights_and_language() {
        for bad in ["vector_weight", "fulltext_weight", "language"] {
            let err = parse_search_args(
                SearchFunction::Knn,
                &[
                    TableFunctionArg::positional(text("q")),
                    TableFunctionArg::named("workspaces", text("library")),
                    TableFunctionArg::named(bad, text("x")),
                ],
                "en",
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("unknown argument"), "{bad}: {err}");
        }
    }

    #[test]
    fn knn_accepts_a_vector_literal_and_an_unwrapped_embedding_call() {
        let a = parse_search_args(
            SearchFunction::Knn,
            &[
                TableFunctionArg::positional(TypedExpr::new(
                    Expr::Literal(Literal::Vector(vec![0.1, 0.2])),
                    DataType::Vector(2),
                )),
                TableFunctionArg::named("workspaces", text("library")),
            ],
            "en",
        )
        .unwrap();
        assert!(matches!(a.query, QueryInput::Vector(ref v) if v.len() == 2));
        assert!(!a.runs_fulltext(), "a raw vector has no lexical surface");
    }

    fn knn_query(arg: TypedExpr) -> Result<SearchArgs, ExecutionError> {
        parse_search_args(
            SearchFunction::Knn,
            &[
                TableFunctionArg::positional(arg),
                TableFunctionArg::named("workspaces", text("library")),
            ],
            "en",
        )
    }

    fn call(name: &str, args: Vec<TypedExpr>) -> TypedExpr {
        use raisin_sql::analyzer::{FunctionCategory, FunctionSignature};
        TypedExpr::new(
            Expr::Function {
                name: name.to_string(),
                args,
                signature: FunctionSignature {
                    name: name.to_string(),
                    params: vec![],
                    return_type: DataType::Unknown,
                    is_deterministic: false,
                    category: FunctionCategory::Vector,
                },
                filter: None,
            },
            DataType::Unknown,
        )
    }

    /// THE bound-parameter path. `$1` never reaches this parser: the HTTP and
    /// pgwire substituters render a bound JSON array into the SQL text as the
    /// quoted string `'[0.1,0.2]'`. Before this it was EMBEDDED as that string
    /// -- a plausible ranking over a nonsense vector, with nothing logged.
    #[test]
    fn a_bound_vector_arrives_as_text_and_is_recognised() {
        let a = knn_query(text("[0.25,-0.5,0.75]")).unwrap();
        match a.query {
            QueryInput::Vector(ref v) => assert_eq!(v, &vec![0.25, -0.5, 0.75]),
            ref other => panic!("bound vector parsed as {other:?}"),
        }
        assert!(!a.runs_fulltext());
    }

    /// ... and prose is untouched, which is the entire cost of the ambiguity.
    #[test]
    fn a_text_query_is_still_a_text_query() {
        let a = knn_query(text("orange tabby cat")).unwrap();
        assert!(matches!(a.query, QueryInput::Text(_)));
        let a = knn_query(text("[draft] release notes")).unwrap();
        assert!(matches!(a.query, QueryInput::Text(_)));
    }

    /// The pgvector text form is a KNN affordance only. HYBRID_SEARCH needs the
    /// text for its full-text half, and silently turning it into a vector would
    /// be a vector-only search reported as hybrid.
    #[test]
    fn hybrid_does_not_reinterpret_a_bracketed_string() {
        let a = hybrid(vec![
            TableFunctionArg::positional(text("[0.25,-0.5]")),
            TableFunctionArg::named("workspaces", text("library")),
        ])
        .unwrap();
        assert!(matches!(a.query, QueryInput::Text(_)));
    }

    #[test]
    fn vector_of_parses_into_an_unresolved_reference() {
        let a = knn_query(call("VECTOR_OF", vec![text("assets:/photos/cat.jpg")])).unwrap();
        match a.query {
            QueryInput::StoredVector(ref r) => {
                assert_eq!(r.workspace, "assets");
                assert_eq!(r.chunk, None);
            }
            ref other => panic!("VECTOR_OF parsed as {other:?}"),
        }
        assert!(!a.runs_fulltext(), "a stored vector has no lexical surface");
        assert!(
            a.query.needs_no_provider(),
            "a stored vector must not require an embedding provider -- that is \
             half the reason it exists"
        );
    }

    #[test]
    fn vector_of_takes_an_optional_chunk_index() {
        let a = knn_query(call(
            "VECTOR_OF",
            vec![text("library:/manuals/boiler"), int(3)],
        ))
        .unwrap();
        match a.query {
            QueryInput::StoredVector(ref r) => assert_eq!(r.chunk, Some(3)),
            ref other => panic!("VECTOR_OF parsed as {other:?}"),
        }
    }

    /// Same rule as a raw vector literal, and for the same reason.
    #[test]
    fn hybrid_refuses_vector_of_and_says_what_to_use() {
        let err = hybrid(vec![
            TableFunctionArg::positional(call("VECTOR_OF", vec![text("assets:/a.jpg")])),
            TableFunctionArg::named("workspaces", text("library")),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("KNN(VECTOR_OF"), "{err}");
    }

    #[test]
    fn vector_of_needs_a_text_reference() {
        let err = knn_query(call("VECTOR_OF", vec![int(7)]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("node reference"), "{err}");
    }
}
