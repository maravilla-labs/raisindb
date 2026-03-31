//! Async expression evaluation
//!
//! This module provides asynchronous expression evaluation, primarily for:
//! - EMBEDDING() function: Calls external APIs with caching
//! - RESOLVE() function: Resolves PropertyValue::Reference to full node data
//! - Any binary operations containing async functions

use crate::physical_plan::executor::Row;
use raisin_core::services::reference_resolver::ReferenceResolver;
use raisin_error::Error;
use raisin_models::nodes::properties::{extract_references, PropertyValue, RaisinReference};
use raisin_sql::analyzer::{BinaryOperator, Expr, Literal, TypedExpr};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use super::core::eval_expr;
use super::helpers::{
    arithmetic_op, compare_literals, is_zero, literals_equal, logical_and, logical_or,
};
use super::vector_ops::{dot_product, extract_vector, l2_distance};

/// Evaluate a typed expression against a row asynchronously
///
/// This async version is needed for functions that require async operations,
/// specifically EMBEDDING() which calls external embedding APIs.
///
/// # Arguments
/// - `expr`: The typed expression to evaluate
/// - `row`: The current row data
/// - `ctx`: Execution context with embedding provider and cache
///
/// # Returns
/// The evaluated literal value, or an error if evaluation fails
pub fn eval_expr_async<'a, S: raisin_storage::Storage + 'a>(
    expr: &'a TypedExpr,
    row: &'a Row,
    ctx: &'a crate::physical_plan::executor::ExecutionContext<S>,
) -> Pin<Box<dyn Future<Output = Result<Literal, Error>> + Send + 'a>> {
    Box::pin(async move {
        match &expr.expr {
            // Handle async functions (EMBEDDING, RESOLVE)
            Expr::Function {
                name,
                args,
                signature: _,
                filter: _,
            } if matches!(
                name.to_uppercase().as_str(),
                "EMBEDDING" | "RESOLVE" | "INVOKE" | "INVOKE_SYNC"
            ) =>
            {
                eval_function_async(name, args, row, ctx).await
            }

            // Handle binary operations that may contain EMBEDDING in nested expressions
            Expr::BinaryOp { left, op, right } => {
                eval_binary_op_async(left, op, right, row, ctx).await
            }

            // For all other expressions, delegate to synchronous evaluator
            _ => eval_expr(expr, row),
        }
    })
}

/// Evaluate a binary operation asynchronously (for operations with EMBEDDING in arguments)
async fn eval_binary_op_async<S: raisin_storage::Storage>(
    left: &TypedExpr,
    op: &BinaryOperator,
    right: &TypedExpr,
    row: &Row,
    ctx: &crate::physical_plan::executor::ExecutionContext<S>,
) -> Result<Literal, Error> {
    // Evaluate both sides asynchronously (in case either contains EMBEDDING)
    let left_val = eval_expr_async(left, row, ctx).await?;
    let right_val = eval_expr_async(right, row, ctx).await?;

    // Now perform the operation using the same logic as synchronous version
    match op {
        // Arithmetic operators
        BinaryOperator::Add => arithmetic_op(&left_val, &right_val, |a, b| a + b),
        BinaryOperator::Subtract => arithmetic_op(&left_val, &right_val, |a, b| a - b),
        BinaryOperator::Multiply => arithmetic_op(&left_val, &right_val, |a, b| a * b),
        BinaryOperator::Divide => {
            if is_zero(&right_val) {
                Err(Error::Validation("Division by zero".to_string()))
            } else {
                arithmetic_op(&left_val, &right_val, |a, b| a / b)
            }
        }
        BinaryOperator::Modulo => arithmetic_op(&left_val, &right_val, |a, b| a % b),

        // Comparison operators
        BinaryOperator::Eq => Ok(Literal::Boolean(literals_equal(&left_val, &right_val)?)),
        BinaryOperator::NotEq => Ok(Literal::Boolean(!literals_equal(&left_val, &right_val)?)),
        BinaryOperator::Lt | BinaryOperator::LtEq | BinaryOperator::Gt | BinaryOperator::GtEq => {
            Ok(Literal::Boolean(compare_literals(
                &left_val, &right_val, *op,
            )?))
        }

        // Logical operators
        BinaryOperator::And => logical_and(&left_val, &right_val),
        BinaryOperator::Or => logical_or(&left_val, &right_val),

        // JSON operators
        BinaryOperator::JsonConcat => {
            // JSONB concatenation - reuse logic from sync version
            match (&left_val, &right_val) {
                (Literal::JsonB(left_obj), Literal::JsonB(right_obj)) => {
                    let mut merged = left_obj.clone();
                    if let (
                        serde_json::Value::Object(left_map),
                        serde_json::Value::Object(right_map),
                    ) = (&mut merged, right_obj)
                    {
                        for (key, value) in right_map.iter() {
                            left_map.insert(key.clone(), value.clone());
                        }
                        Ok(Literal::JsonB(merged))
                    } else {
                        Ok(Literal::JsonB(right_obj.clone()))
                    }
                }
                (Literal::Null, Literal::JsonB(obj)) | (Literal::JsonB(obj), Literal::Null) => {
                    Ok(Literal::JsonB(obj.clone()))
                }
                (Literal::Null, Literal::Null) => Ok(Literal::Null),
                _ => Err(Error::Validation(
                    "JSON concatenation (||) requires JSONB operands".to_string(),
                )),
            }
        }

        // String concatenation: Text || Text → Text
        BinaryOperator::StringConcat => {
            match (&left_val, &right_val) {
                // NULL handling: NULL || anything = NULL (PostgreSQL semantics)
                (Literal::Null, _) | (_, Literal::Null) => Ok(Literal::Null),
                // String concatenation variants
                (Literal::Text(l), Literal::Text(r)) => Ok(Literal::Text(format!("{}{}", l, r))),
                (Literal::Text(l), Literal::Path(r)) => Ok(Literal::Text(format!("{}{}", l, r))),
                (Literal::Path(l), Literal::Text(r)) => Ok(Literal::Text(format!("{}{}", l, r))),
                (Literal::Path(l), Literal::Path(r)) => Ok(Literal::Text(format!("{}{}", l, r))),
                // Coerce other types to string (PostgreSQL allows this)
                (l, r) => Ok(Literal::Text(format!(
                    "{}{}",
                    literal_to_string(l),
                    literal_to_string(r)
                ))),
            }
        }

        BinaryOperator::JsonExtract | BinaryOperator::JsonContains => Err(Error::Validation(
            "JSON operators should be handled at expression level".to_string(),
        )),

        // Full-text search operator
        BinaryOperator::TextSearchMatch => Err(Error::Validation(
            "Text search operator @@ should be handled at query planning level".to_string(),
        )),

        // Vector distance operators
        // Handle NULL values: if either vector is NULL, distance is NULL
        BinaryOperator::VectorL2Distance => {
            if matches!(left_val, Literal::Null) || matches!(right_val, Literal::Null) {
                return Ok(Literal::Null);
            }
            let v1 = extract_vector(&left_val)?;
            let v2 = extract_vector(&right_val)?;
            let distance = l2_distance(&v1, &v2);
            Ok(Literal::Double(distance as f64))
        }
        BinaryOperator::VectorCosineDistance => {
            if matches!(left_val, Literal::Null) || matches!(right_val, Literal::Null) {
                return Ok(Literal::Null);
            }
            let v1 = extract_vector(&left_val)?;
            let v2 = extract_vector(&right_val)?;
            let distance = 1.0 - dot_product(&v1, &v2);
            Ok(Literal::Double(distance as f64))
        }
        BinaryOperator::VectorInnerProduct => {
            if matches!(left_val, Literal::Null) || matches!(right_val, Literal::Null) {
                return Ok(Literal::Null);
            }
            let v1 = extract_vector(&left_val)?;
            let v2 = extract_vector(&right_val)?;
            let product = -dot_product(&v1, &v2);
            Ok(Literal::Double(product as f64))
        }
    }
}

/// Generate embedding with caching
///
/// This is a helper function that encapsulates the embedding generation + caching logic.
/// It can be called from both eval_function_async and execute_vector_scan.
///
/// # Caching Strategy
/// 1. Check in-memory cache first
/// 2. If not found, call embedding provider API
/// 3. Store result in cache for future queries
///
/// # Arguments
/// * `text` - The input text to generate an embedding for
/// * `ctx` - Execution context containing the cache and embedding provider
pub async fn generate_embedding_cached<S: raisin_storage::Storage>(
    text: &str,
    ctx: &crate::physical_plan::executor::ExecutionContext<S>,
) -> Result<Vec<f32>, Error> {
    // Check cache first
    {
        let mut cache = ctx.embedding_cache.write().await;

        // Check if we have a cached entry
        if let Some(cached) = cache.get(text) {
            // Check if it's still valid (within TTL)
            if !cached.is_expired(ctx.embedding_cache_ttl) {
                // Performance: Use lazy tracing to avoid string allocations
                if tracing::enabled!(tracing::Level::DEBUG) {
                    tracing::debug!(
                        "✓ EMBEDDING cache hit for: {} (age: {:.1}s)",
                        &text[..text.len().min(50)],
                        cached.cached_at.elapsed().as_secs_f32()
                    );
                }
                return Ok(cached.vector.clone());
            } else {
                // Expired, remove it
                if tracing::enabled!(tracing::Level::DEBUG) {
                    tracing::debug!(
                        "⚠️  EMBEDDING cache expired for: {} (age: {:.1}s > TTL: {:.1}s)",
                        &text[..text.len().min(50)],
                        cached.cached_at.elapsed().as_secs_f32(),
                        ctx.embedding_cache_ttl.as_secs_f32()
                    );
                }
                cache.remove(text);
            }
        }
    }

    // Not in cache or expired - generate it
    let provider = ctx.embedding_provider.as_ref().ok_or_else(|| {
        Error::Validation(
            "EMBEDDING() function requires an embedding provider to be configured".to_string(),
        )
    })?;

    // Performance: Use lazy tracing and string slicing instead of collecting chars
    if tracing::enabled!(tracing::Level::INFO) {
        tracing::info!(
            "🔮 Generating embedding for: {}",
            &text[..text.len().min(50)]
        );
    }
    let embedding = provider
        .generate_embedding(text)
        .await
        .map_err(|e| Error::Backend(format!("Failed to generate embedding: {}", e)))?;

    // Store in cache with current timestamp
    {
        let mut cache = ctx.embedding_cache.write().await;
        cache.insert(
            text.to_string(),
            crate::physical_plan::executor::CachedEmbedding::new(embedding.clone()),
        );
    }

    tracing::debug!(
        "✓ Cached embedding (dimensions: {}, TTL: {}s)",
        embedding.len(),
        ctx.embedding_cache_ttl.as_secs()
    );
    Ok(embedding)
}

/// Evaluate a function call asynchronously (for functions requiring async operations)
///
/// Currently handles:
/// - EMBEDDING(text): Generates embedding vector via external API with caching
/// - RESOLVE(jsonb[, depth]): Resolves PropertyValue::Reference to full node data
///
/// # Caching Strategy
/// EMBEDDING results are cached in ExecutionContext.embedding_cache to avoid
/// redundant API calls for the same input text. Cache is shared across all
/// operators in the query execution.
async fn eval_function_async<S: raisin_storage::Storage>(
    name: &str,
    args: &[TypedExpr],
    row: &Row,
    ctx: &crate::physical_plan::executor::ExecutionContext<S>,
) -> Result<Literal, Error> {
    match name.to_uppercase().as_str() {
        "EMBEDDING" => {
            // EMBEDDING(text) - generate embedding vector for text
            if args.len() != 1 {
                return Err(Error::Validation(
                    "EMBEDDING requires exactly 1 argument".to_string(),
                ));
            }

            // Evaluate the argument to get the text
            let text_lit = eval_expr(&args[0], row)?;
            let text = match text_lit {
                Literal::Text(s) => s,
                _ => {
                    return Err(Error::Validation(
                        "EMBEDDING requires a text argument".to_string(),
                    ))
                }
            };

            // Use the shared cached embedding generation function
            let embedding = generate_embedding_cached(&text, ctx).await?;
            Ok(Literal::Vector(embedding))
        }

        "RESOLVE" => eval_resolve(args, row, ctx).await,
        "INVOKE" => eval_invoke(args, row, ctx).await,
        "INVOKE_SYNC" => eval_invoke_sync(args, row, ctx).await,

        _ => Err(Error::Validation(format!(
            "Unknown async function: {}",
            name
        ))),
    }
}

/// Evaluate RESOLVE(jsonb[, depth]) - resolve PropertyValue::Reference to full node data
///
/// Two code paths:
/// - Single reference: Input JSON has "raisin:ref" key -> fetch that one node
/// - Full properties: Input JSON is a properties object -> walk and replace all references
///
/// Returns JSONB with references replaced by full node objects.
/// Returns NULL if input is NULL. Unresolvable references are kept as-is.
async fn eval_resolve<S: raisin_storage::Storage>(
    args: &[TypedExpr],
    row: &Row,
    ctx: &crate::physical_plan::executor::ExecutionContext<S>,
) -> Result<Literal, Error> {
    if args.is_empty() || args.len() > 2 {
        return Err(Error::Validation(
            "RESOLVE requires 1 or 2 arguments: RESOLVE(jsonb[, depth])".to_string(),
        ));
    }

    // Evaluate the JSONB argument
    let json_lit = eval_expr_async(&args[0], row, ctx).await?;

    // Handle NULL input
    if matches!(json_lit, Literal::Null) {
        return Ok(Literal::Null);
    }

    let json_value = match json_lit {
        Literal::JsonB(v) => v,
        _ => {
            return Err(Error::Validation(
                "RESOLVE first argument must be JSONB".to_string(),
            ))
        }
    };

    // Parse optional depth argument (default: 1, max: 10)
    let max_depth = if args.len() == 2 {
        let depth_lit = eval_expr(&args[1], row)?;
        match depth_lit {
            Literal::Null => 1, // NULL depth treated as default
            Literal::Int(d) if d < 0 => {
                return Err(Error::Validation(
                    "RESOLVE depth must be non-negative".to_string(),
                ))
            }
            Literal::Int(d) => (d as u32).min(10),
            Literal::BigInt(d) if d < 0 => {
                return Err(Error::Validation(
                    "RESOLVE depth must be non-negative".to_string(),
                ))
            }
            Literal::BigInt(d) => (d as u32).min(10),
            _ => {
                return Err(Error::Validation(
                    "RESOLVE depth argument must be an integer".to_string(),
                ))
            }
        }
    } else {
        1
    };

    if max_depth == 0 {
        return Ok(Literal::JsonB(json_value));
    }

    let workspace = ctx.workspace.as_ref();

    // Path A: Single reference (JSON object with "raisin:ref" key)
    if json_value.get("raisin:ref").is_some() {
        let reference: RaisinReference =
            serde_json::from_value(json_value.clone()).map_err(|e| {
                Error::Validation(format!("Failed to parse reference from JSONB: {}", e))
            })?;

        let resolver = ReferenceResolver::new(
            ctx.storage.clone(),
            ctx.tenant_id.to_string(),
            ctx.repo_id.to_string(),
            ctx.branch.to_string(),
        );

        match resolver
            .resolve_single_reference(workspace, &reference, max_depth)
            .await
            .map_err(|e| Error::Backend(format!("RESOLVE() storage error: {}", e)))?
        {
            Some(resolved_json) => Ok(Literal::JsonB(resolved_json)),
            None => {
                // Unresolvable reference: keep original
                Ok(Literal::JsonB(json_value))
            }
        }
    }
    // Path B: Full properties object - walk and replace all references
    else {
        // Deserialize JSON to HashMap<String, PropertyValue>
        let properties: HashMap<String, PropertyValue> = serde_json::from_value(json_value.clone())
            .map_err(|e| {
                Error::Validation(format!(
                    "Failed to parse properties from JSONB for RESOLVE: {}",
                    e
                ))
            })?;

        // Check if there are any references to resolve
        let refs = extract_references(&properties);
        if refs.is_empty() {
            return Ok(Literal::JsonB(json_value));
        }

        let resolver = ReferenceResolver::new(
            ctx.storage.clone(),
            ctx.tenant_id.to_string(),
            ctx.repo_id.to_string(),
            ctx.branch.to_string(),
        );

        let resolved_properties = resolver
            .resolve_properties(workspace, &properties, max_depth)
            .await
            .map_err(|e| Error::Backend(format!("RESOLVE() storage error: {}", e)))?;

        let result_json = serde_json::to_value(&resolved_properties).map_err(|e| {
            Error::Internal(format!("Failed to serialize resolved properties: {}", e))
        })?;

        Ok(Literal::JsonB(result_json))
    }
}

/// Evaluate INVOKE(path[, input[, workspace]]) - queue a background function invocation.
/// Returns JSONB with execution_id and job_id.
async fn eval_invoke<S: raisin_storage::Storage>(
    args: &[TypedExpr],
    row: &Row,
    ctx: &crate::physical_plan::executor::ExecutionContext<S>,
) -> Result<Literal, Error> {
    let (path, input, workspace) = parse_invoke_args(args, row, ctx).await?;
    let cb = ctx.function_invoke.as_ref().ok_or_else(|| {
        Error::Validation(
            "INVOKE() requires function invocation support (not available in this context)"
                .to_string(),
        )
    })?;
    let (execution_id, job_id) = cb(path, input, workspace).await?;
    Ok(Literal::JsonB(serde_json::json!({
        "execution_id": execution_id,
        "job_id": job_id,
    })))
}

/// Evaluate INVOKE_SYNC(path[, input[, workspace]]) - execute a function inline.
/// Returns the function result as JSONB.
async fn eval_invoke_sync<S: raisin_storage::Storage>(
    args: &[TypedExpr],
    row: &Row,
    ctx: &crate::physical_plan::executor::ExecutionContext<S>,
) -> Result<Literal, Error> {
    let (path, input, workspace) = parse_invoke_args(args, row, ctx).await?;
    let cb = ctx.function_invoke_sync.as_ref().ok_or_else(|| {
        Error::Validation(
            "INVOKE_SYNC() requires function invocation support (not available in this context)"
                .to_string(),
        )
    })?;
    let result = cb(path, input, workspace).await?;
    Ok(Literal::JsonB(result))
}

/// Shared argument parsing for INVOKE/INVOKE_SYNC: (path[, input[, workspace]])
async fn parse_invoke_args<S: raisin_storage::Storage>(
    args: &[TypedExpr],
    row: &Row,
    ctx: &crate::physical_plan::executor::ExecutionContext<S>,
) -> Result<(String, serde_json::Value, Option<String>), Error> {
    if args.is_empty() || args.len() > 3 {
        return Err(Error::Validation(
            "INVOKE requires 1-3 arguments: INVOKE(path[, input[, workspace]])".to_string(),
        ));
    }
    // Arg 1: function path (required)
    let path_lit = eval_expr_async(&args[0], row, ctx).await?;
    let path = match path_lit {
        Literal::Text(s) | Literal::Path(s) => s,
        _ => {
            return Err(Error::Validation(
                "INVOKE first argument must be a text path".to_string(),
            ))
        }
    };
    // Arg 2: input (optional, defaults to {})
    let input = if args.len() >= 2 {
        let input_lit = eval_expr_async(&args[1], row, ctx).await?;
        match input_lit {
            Literal::JsonB(v) => v,
            Literal::Null => serde_json::Value::Object(Default::default()),
            _ => {
                return Err(Error::Validation(
                    "INVOKE second argument must be JSONB".to_string(),
                ))
            }
        }
    } else {
        serde_json::Value::Object(Default::default())
    };
    // Arg 3: workspace (optional)
    let workspace = if args.len() == 3 {
        let ws_lit = eval_expr_async(&args[2], row, ctx).await?;
        match ws_lit {
            Literal::Text(s) => Some(s),
            Literal::Null => None,
            _ => {
                return Err(Error::Validation(
                    "INVOKE third argument must be a text workspace name".to_string(),
                ))
            }
        }
    } else {
        None
    };
    Ok((path, input, workspace))
}

/// Convert a Literal to its string representation for string concatenation
fn literal_to_string(lit: &Literal) -> String {
    match lit {
        Literal::Null => String::new(), // Should not reach here due to NULL handling above
        Literal::Boolean(b) => b.to_string(),
        Literal::Int(i) => i.to_string(),
        Literal::BigInt(i) => i.to_string(),
        Literal::Double(f) => f.to_string(),
        Literal::Text(s) => s.clone(),
        Literal::Uuid(s) => s.clone(),
        Literal::Path(s) => s.clone(),
        Literal::JsonB(v) => v.to_string(),
        Literal::Vector(v) => format!("{:?}", v),
        Literal::Geometry(v) => v.to_string(),
        Literal::Timestamp(ts) => ts.to_rfc3339(),
        Literal::Interval(d) => format!("{}s", d.num_seconds()),
        Literal::Parameter(p) => p.clone(),
    }
}
