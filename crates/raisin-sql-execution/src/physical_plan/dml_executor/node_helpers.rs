// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node-specific helper functions for workspace DML operations.
//!
//! Contains functions for building nodes from column maps,
//! applying assignments to nodes, and filter classification.

use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{BinaryOperator, Expr, Literal, TypedExpr};

use super::helpers::{extract_optional_string_column, extract_string_column, extract_string_value};

/// Node identifier extracted from WHERE clause.
#[derive(Debug, Clone)]
pub enum NodeIdentifier {
    Id(String),
    Path(String),
}

/// Filter complexity classification for determining sync vs async execution.
#[derive(Debug, Clone)]
pub enum FilterComplexity {
    /// Simple filter: WHERE id = 'xxx' or WHERE path = '/xxx'
    Simple(NodeIdentifier),
    /// Complex filter requiring full table scan
    Complex,
    /// No filter provided
    None,
}

/// Classify a WHERE clause filter to determine execution strategy.
pub fn classify_filter(filter: &Option<TypedExpr>) -> FilterComplexity {
    match filter {
        None => FilterComplexity::None,
        Some(_) => match extract_node_identifier_from_filter(filter) {
            Ok(ident) => FilterComplexity::Simple(ident),
            Err(_) => FilterComplexity::Complex,
        },
    }
}

/// Extract node identifier (id or path) from WHERE clause filter.
pub(super) fn extract_node_identifier_from_filter(
    filter: &Option<TypedExpr>,
) -> Result<NodeIdentifier, Error> {
    let filter_expr = filter.as_ref().ok_or_else(|| {
        Error::Validation(
            "UPDATE/DELETE on workspace tables requires a WHERE clause with id or path (e.g., WHERE id = '...' or WHERE path = '...')".to_string()
        )
    })?;

    if let Expr::BinaryOp {
        left,
        op: BinaryOperator::Eq,
        right,
    } = &filter_expr.expr
    {
        if let Expr::Column { column, .. } = &left.expr {
            if column == "id" {
                if let Expr::Literal(Literal::Text(id_value)) = &right.expr {
                    return Ok(NodeIdentifier::Id(id_value.clone()));
                }
            } else if column == "path" {
                match &right.expr {
                    Expr::Literal(Literal::Text(path_value))
                    | Expr::Literal(Literal::Path(path_value)) => {
                        return Ok(NodeIdentifier::Path(path_value.clone()));
                    }
                    _ => {}
                }
            }
        }
    }

    Err(Error::Validation(
        "UPDATE/DELETE WHERE clause must be a simple equality: WHERE id = 'value' or WHERE path = '/value'".to_string()
    ))
}

/// Build a Node from column->value map.
pub(super) fn build_node_from_columns(
    col_map: &IndexMap<String, PropertyValue>,
    workspace: &str,
    actor: Option<&str>,
) -> Result<raisin_models::nodes::Node, Error> {
    use raisin_models::nodes::Node;

    let path = extract_string_column(col_map, "path")?;
    let node_type = extract_string_column(col_map, "node_type")?;

    let id = extract_optional_string_column(col_map, "id")
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let name = path.rsplit('/').next().unwrap_or(&path).to_string();

    let reserved_columns = [
        "id",
        "path",
        "name",
        "node_type",
        "archetype",
        "version",
        "workspace",
        "properties",
    ];
    let mut properties = std::collections::HashMap::new();

    // Unpack properties column if present
    if let Some(PropertyValue::Object(props_obj)) = col_map.get("properties") {
        properties = props_obj.clone();
    }

    // Add all other non-reserved columns as individual properties
    for (key, value) in col_map.iter() {
        if !reserved_columns.contains(&key.as_str()) {
            properties.insert(key.clone(), value.clone());
        }
    }

    let now = chrono::Utc::now();

    Ok(Node {
        id,
        name,
        path,
        node_type,
        archetype: extract_optional_string_column(col_map, "archetype"),
        properties,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: None,
        version: 1,
        created_at: Some(now),
        updated_at: Some(now),
        published_at: None,
        published_by: None,
        updated_by: Some(actor.unwrap_or("sql-insert").to_string()),
        created_by: Some(actor.unwrap_or("sql-insert").to_string()),
        translations: None,
        tenant_id: None,
        workspace: Some(workspace.to_string()),
        owner_id: None,
        relations: vec![],
    })
}

/// Apply assignment to Node field.
pub(super) fn apply_assignment_to_node(
    node: &mut raisin_models::nodes::Node,
    col_name: &str,
    value: PropertyValue,
) -> Result<(), Error> {
    match col_name {
        "name" => {
            node.name = extract_string_value(&value)?;
        }
        "path" => {
            node.path = extract_string_value(&value)?;
        }
        "node_type" => {
            return Err(Error::Validation(
                "Cannot change node_type after creation".to_string(),
            ));
        }
        "archetype" => {
            node.archetype = Some(extract_string_value(&value)?);
        }
        "properties" => {
            tracing::debug!("apply_assignment_to_node: handling 'properties' column assignment");
            if let PropertyValue::Object(mut obj) = value {
                // Recursively flatten any nested "properties" keys
                let mut flatten_count = 0;
                while let Some(PropertyValue::Object(nested)) = obj.remove("properties") {
                    flatten_count += 1;
                    tracing::warn!(
                        "Flattening nested 'properties' object (level {}) with {} keys into top level",
                        flatten_count,
                        nested.len()
                    );
                    for (k, v) in nested {
                        obj.insert(k, v);
                    }
                }
                if flatten_count > 0 {
                    tracing::info!(
                        "Flattened {} levels of nested 'properties' objects",
                        flatten_count
                    );
                }
                tracing::debug!(
                    "Replacing node.properties with {} keys: {:?}",
                    obj.len(),
                    obj.keys().collect::<Vec<_>>()
                );
                node.properties = obj;
            } else {
                tracing::error!(
                    "properties column assignment received non-Object value: {:?}",
                    value
                );
                return Err(Error::Validation(
                    "properties column must be a JSON object".to_string(),
                ));
            }
        }
        _ => {
            if col_name.eq_ignore_ascii_case("properties") {
                tracing::error!(
                    "Column name '{}' matched catch-all but should have matched 'properties' case.",
                    col_name
                );
            }
            tracing::trace!("Inserting property key '{}' into node.properties", col_name);
            node.properties.insert(col_name.to_string(), value);
        }
    }
    Ok(())
}
