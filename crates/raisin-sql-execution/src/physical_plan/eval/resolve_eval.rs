//! `RESOLVE(jsonb[, depth[, fields]])` — inline referenced nodes.
//!
//! The engine lives in `raisin_core::services::reference_resolver`; this is
//! the SQL binding. What it adds is the STATEMENT: one snapshot every target
//! is read at, one memo every row shares, one translation resolver, and the
//! caller's identity, against which every target is checked.

use crate::physical_plan::executor::{ExecutionContext, Row};
use raisin_core::services::reference_resolver::{ReferenceResolver, ResolutionLocale};
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::LocaleCode;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::async_eval::eval_expr_async;
use super::core::eval_expr;

/// The locale RESOLVE() should read referenced nodes in.
///
/// The scan executors have already applied the query's locale to the row this
/// expression is evaluating over (`ctx.locales`, extracted from the
/// `locale = '<x>'` predicate before it ever reaches the filter). A reference is
/// part of that same document, so it is read in that same language — otherwise a
/// translated page inlines untranslated nodes and is silently half translated.
///
/// WHICH locale, when the query asked for several: a `locale IN ('ja','de')`
/// read fans out one ROW PER LOCALE, so the locale is a property of the row and
/// not of the query. The row carries it as the virtual `locale` column — when
/// that column is projected we use it, which is exact. When it is not projected
/// there is nothing on the row to read and we fall back to the first requested
/// locale, which is exact for the single-locale case that every translated site
/// actually issues, and is at worst the previous behaviour for one of the rows
/// of a multi-locale fan-out that did not ask to see its own locale.
///
/// `None` — meaning "read the base language" — whenever the repository has no
/// translation configuration, the requested locale IS the default language, or
/// the code does not parse.
fn resolution_locale<S: raisin_storage::Storage>(
    row: &Row,
    ctx: &ExecutionContext<S>,
) -> Option<ResolutionLocale> {
    let config = ctx.repository_config.as_ref()?;

    // The row's own locale wins over the query's list; see above. Looked up
    // unqualified because the column is `<qualifier>.locale` and this expression
    // does not know the qualifier it is being evaluated under.
    let from_row = match row.get_by_unqualified("locale") {
        Some(PropertyValue::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let wanted = from_row
        .or_else(|| ctx.locales.first().cloned())
        .unwrap_or_else(|| ctx.default_language.to_string());

    if wanted == ctx.default_language.as_ref() {
        return None;
    }

    Some(ResolutionLocale {
        locale: LocaleCode::parse(&wanted).ok()?,
        default_language: ctx.default_language.to_string(),
        config: config.clone(),
    })
}

/// Evaluate RESOLVE(jsonb[, depth[, fields]]) - inline referenced nodes.
///
/// The argument is either a single reference (`{"raisin:ref": ...}`), which
/// resolves to the node, or any JSON value (typically `properties`) whose
/// references are replaced by the nodes they point to. NULL in is NULL out.
///
/// A reference stays as written when its target is missing at the statement's
/// snapshot, hidden in the row's locale, OR not readable by the caller — the
/// three are deliberately indistinguishable, so RESOLVE cannot be used to probe
/// for nodes the caller may not see. There is no opt-out.
///
/// `fields` is a comma-separated list of property names (`'title,file,alt'`).
/// When given, every inlined node carries only `id`, `name`, `path`,
/// `node_type` and those properties — what a renderer actually reads of an
/// asset, instead of the whole node.
pub(super) async fn eval_resolve<S: raisin_storage::Storage>(
    args: &[TypedExpr],
    row: &Row,
    ctx: &ExecutionContext<S>,
) -> Result<Literal, Error> {
    if args.is_empty() || args.len() > 3 {
        return Err(Error::Validation(
            "RESOLVE requires 1 to 3 arguments: RESOLVE(jsonb[, depth[, fields]])".to_string(),
        ));
    }

    let json_value = match eval_expr_async(&args[0], row, ctx).await? {
        Literal::Null => return Ok(Literal::Null),
        Literal::JsonB(v) => v,
        _ => {
            return Err(Error::Validation(
                "RESOLVE first argument must be JSONB".to_string(),
            ))
        }
    };

    // Optional depth argument (default: 1, max: 10)
    let max_depth = if args.len() >= 2 {
        match eval_expr(&args[1], row)? {
            Literal::Null => 1, // NULL depth treated as default
            Literal::Int(d) if d < 0 => return Err(negative_depth()),
            Literal::Int(d) => (d as u32).min(10),
            Literal::BigInt(d) if d < 0 => return Err(negative_depth()),
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

    let fields = if args.len() == 3 {
        match eval_expr(&args[2], row)? {
            Literal::Null => None,
            Literal::Text(list) => Some(parse_resolve_fields(&list)),
            _ => {
                return Err(Error::Validation(
                    "RESOLVE fields argument must be a comma-separated text list".to_string(),
                ))
            }
        }
    } else {
        None
    };

    if max_depth == 0 {
        return Ok(Literal::JsonB(json_value));
    }

    let mut resolver = ReferenceResolver::new(
        ctx.storage.clone(),
        ctx.tenant_id.to_string(),
        ctx.repo_id.to_string(),
        ctx.branch.to_string(),
        ctx.statement_snapshot().await?,
    )
    .with_auth(ctx.auth_context.clone())
    .with_memo(ctx.resolve_memo())
    // The language this read is in — see `resolution_locale` for why RESOLVE()
    // has to know it, and for the one case it cannot answer exactly.
    .with_locale(resolution_locale(row, ctx));
    if let Some(translations) = ctx.translation_resolver() {
        resolver = resolver.with_translation_resolver(translations);
    }

    let resolved = resolver
        .resolve_json(
            ctx.workspace.as_ref(),
            &json_value,
            max_depth,
            fields.as_deref(),
        )
        .await
        .map_err(|e| match e {
            // Budget and argument errors already name RESOLVE.
            Error::Validation(_) => e,
            other => Error::Backend(format!("RESOLVE() storage error: {}", other)),
        })?;

    Ok(Literal::JsonB(resolved))
}

fn negative_depth() -> Error {
    Error::Validation("RESOLVE depth must be non-negative".to_string())
}

/// `'title, file ,alt'` -> `["title", "file", "alt"]`; empty entries dropped.
fn parse_resolve_fields(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect()
}
