//! Writes to the reserved `Workspaces` schema table.
//!
//! A workspace definition is not a bare row: creating one also builds its nodes
//! table, bootstraps its ROOT node and seeds `initial_structure`. That work
//! lives in `WorkspaceService::put`, the same call the management API
//! (`PUT /api/workspaces/{repo}/{name}`) makes, so SQL goes through it too and
//! the two doors cannot produce different workspaces.
//!
//! Only the definition's own fields are writable: `name` (INSERT only),
//! `description`, `allowed_node_types`, `allowed_root_node_types`, `depends_on`.
//! Configuration (default branch, …) stays with the management API.

use crate::physical_plan::executor::ExecutionContext;
use indexmap::IndexMap;
use raisin_core::WorkspaceService;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::workspace::Workspace;
use raisin_sql::analyzer::TypedExpr;
use raisin_storage::Storage;

use super::helpers::{eval_expr_to_property_value, extract_string_array};

const WRITABLE: &[&str] = &[
    "name",
    "description",
    "allowed_node_types",
    "allowed_root_node_types",
    "depends_on",
];

/// A workspace name is a URL segment and a SQL table name. Lowercase letters,
/// digits and `_`, starting with a letter; `raisin:`-style system names are
/// never created here.
fn check_name(name: &str) -> Result<(), Error> {
    let ok = name.len() <= 64
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Error::Validation(format!(
            "Workspace name '{name}' is not valid: use lowercase letters, digits and '_', starting with a letter (at most 64)"
        )))
    }
}

/// Strings in an array, whether the value arrived as an array or as the JSON
/// text of one (`'["a:B"]'`).
fn string_list(column: &str, value: &PropertyValue) -> Result<Vec<String>, Error> {
    if let PropertyValue::String(text) = value {
        return serde_json::from_str::<Vec<String>>(text).map_err(|e| {
            Error::Validation(format!(
                "Column '{column}' must be an array of strings: {e}"
            ))
        });
    }
    extract_string_array(value).map_err(|e| Error::Validation(format!("Column '{column}': {e}")))
}

fn apply(ws: &mut Workspace, column: &str, value: &PropertyValue) -> Result<(), Error> {
    match column {
        "description" => {
            ws.description = match value {
                PropertyValue::Null => None,
                PropertyValue::String(s) => Some(s.clone()),
                _ => {
                    return Err(Error::Validation(
                        "Column 'description' must be a string".into(),
                    ))
                }
            }
        }
        "allowed_node_types" => ws.allowed_node_types = string_list(column, value)?,
        "allowed_root_node_types" => ws.allowed_root_node_types = string_list(column, value)?,
        "depends_on" => ws.depends_on = string_list(column, value)?,
        other => {
            return Err(Error::Validation(format!(
                "Column '{other}' of Workspaces is not writable over SQL; writable: {}",
                WRITABLE.join(", ")
            )))
        }
    }
    Ok(())
}

/// Root types must also be allowed types, or nothing could be created at the
/// root that the workspace then accepts.
fn check_roots(ws: &Workspace) -> Result<(), Error> {
    if ws.allowed_node_types.is_empty() || ws.allowed_node_types.iter().any(|t| t == "*") {
        return Ok(());
    }
    let missing: Vec<&String> = ws
        .allowed_root_node_types
        .iter()
        .filter(|t| !ws.allowed_node_types.contains(t))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Error::Validation(format!(
            "allowed_root_node_types {missing:?} must also be in allowed_node_types"
        )))
    }
}

pub(super) async fn execute_insert_workspaces<
    S: Storage + raisin_storage::transactional::TransactionalStorage,
>(
    columns: &[String],
    values: &[Vec<TypedExpr>],
    ctx: &ExecutionContext<S>,
) -> Result<(), Error> {
    let service = WorkspaceService::new(ctx.storage.clone());
    for row in values {
        let mut cols: IndexMap<String, PropertyValue> = IndexMap::new();
        for (col, expr) in columns.iter().zip(row.iter()) {
            cols.insert(col.to_lowercase(), eval_expr_to_property_value(expr)?);
        }
        let name = match cols.get("name") {
            Some(PropertyValue::String(s)) => s.clone(),
            _ => {
                return Err(Error::Validation(
                    "INSERT INTO Workspaces requires a string 'name'".into(),
                ))
            }
        };
        check_name(&name)?;
        if service
            .get(&ctx.tenant_id, &ctx.repo_id, &name)
            .await?
            .is_some()
        {
            return Err(Error::Validation(format!(
                "Workspace '{name}' already exists; change it with UPDATE Workspaces … WHERE name = '{name}'"
            )));
        }
        let mut ws = Workspace::new(name);
        for (col, value) in &cols {
            if col != "name" {
                apply(&mut ws, col, value)?;
            }
        }
        check_roots(&ws)?;
        service.put(&ctx.tenant_id, &ctx.repo_id, ws).await?;
    }
    Ok(())
}

pub(super) async fn execute_update_workspaces<
    S: Storage + raisin_storage::transactional::TransactionalStorage,
>(
    name: &str,
    assignments: &[(String, TypedExpr)],
    ctx: &ExecutionContext<S>,
) -> Result<usize, Error> {
    let service = WorkspaceService::new(ctx.storage.clone());
    let mut ws = service
        .get(&ctx.tenant_id, &ctx.repo_id, name)
        .await?
        .ok_or_else(|| Error::Validation(format!("Workspace '{name}' not found for UPDATE")))?;
    for (col, expr) in assignments {
        let col = col.to_lowercase();
        if col == "name" {
            return Err(Error::Validation(
                "A workspace cannot be renamed over SQL".into(),
            ));
        }
        apply(&mut ws, &col, &eval_expr_to_property_value(expr)?)?;
    }
    check_roots(&ws)?;
    ws.updated_at = Some(raisin_models::timestamp::StorageTimestamp::now());
    service.put(&ctx.tenant_id, &ctx.repo_id, ws).await?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_url_and_table_safe() {
        assert!(check_name("local_crm").is_ok());
        for bad in ["", "Local", "1crm", "raisin:x", "a-b", "a b"] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_list_may_arrive_as_json_text() {
        let v = PropertyValue::String(r#"["local:Deal","raisin:Folder"]"#.into());
        assert_eq!(
            string_list("allowed_node_types", &v).unwrap(),
            vec!["local:Deal", "raisin:Folder"]
        );
    }

    #[test]
    fn a_root_type_must_be_allowed() {
        let mut ws = Workspace::new("local_crm".into());
        ws.allowed_node_types = vec!["local:Deal".into()];
        ws.allowed_root_node_types = vec!["local:Contact".into()];
        assert!(check_roots(&ws).is_err());
        ws.allowed_root_node_types = vec!["local:Deal".into()];
        assert!(check_roots(&ws).is_ok());
    }
}
