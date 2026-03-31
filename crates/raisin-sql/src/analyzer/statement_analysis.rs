//! Statement-specific analysis methods for the Analyzer
//!
//! This module contains the analysis logic for various SQL statement types:
//! - ORDER statements
//! - MOVE statements
//! - COPY statements
//! - RESTORE statements
//! - TRANSLATE statements
//! - RELATE/UNRELATE statements
//!
//! Each analysis function validates the statement's semantic correctness and
//! produces an analyzed representation suitable for execution.

use crate::ast::copy_stmt::CopyStatement;
use crate::ast::move_stmt::MoveStatement;
use crate::ast::order::{NodeReference, OrderStatement};
use crate::ast::relate::{RelateNodeReference, RelateStatement, UnrelateStatement};
use crate::ast::restore::RestoreStatement;
use crate::ast::translate::{
    TranslateFilter, TranslateStatement, TranslationPath, TranslationValue,
};

use super::catalog::Catalog;
use super::error::{AnalysisError, Result};
use super::semantic::{
    AnalyzedCopy, AnalyzedMove, AnalyzedOrder, AnalyzedRelate, AnalyzedRelateEndpoint,
    AnalyzedRestore, AnalyzedTranslate, AnalyzedTranslateFilter, AnalyzedTranslationValue,
    AnalyzedUnrelate,
};

use std::collections::HashMap;

/// Analyze an ORDER statement
///
/// Validates the node references (path format, ID format, self-reference check)
/// and resolves the table name to a workspace.
pub(super) fn analyze_order(catalog: &dyn Catalog, stmt: &OrderStatement) -> Result<AnalyzedOrder> {
    // Resolve table name to workspace
    let workspace = catalog
        .resolve_workspace_name(&stmt.table)
        .ok_or_else(|| AnalysisError::TableNotFound(stmt.table.clone()))?;

    // Validate source reference
    validate_node_reference(&stmt.source)?;

    // Validate target reference
    validate_node_reference(&stmt.target)?;

    // Check for self-reference (same path or same ID)
    match (&stmt.source, &stmt.target) {
        (NodeReference::Path(s), NodeReference::Path(t)) if s == t => {
            return Err(AnalysisError::OrderSelfReference);
        }
        (NodeReference::Id(s), NodeReference::Id(t)) if s == t => {
            return Err(AnalysisError::OrderSelfReference);
        }
        _ => {
            // Mixed path/id - self-reference check happens at execution time
        }
    }

    // Validate siblings - check if both paths have the same parent
    // This can only be done statically when both are path references
    if let (NodeReference::Path(source_path), NodeReference::Path(target_path)) =
        (&stmt.source, &stmt.target)
    {
        let source_parent = get_parent_path(source_path);
        let target_parent = get_parent_path(target_path);

        if source_parent != target_parent {
            return Err(AnalysisError::OrderNotSiblings {
                source_path: source_path.clone(),
                target_path: target_path.clone(),
            });
        }
    }

    Ok(AnalyzedOrder {
        table: stmt.table.clone(),
        workspace,
        source: stmt.source.clone(),
        position: stmt.position,
        target: stmt.target.clone(),
        branch_override: stmt.branch.clone(),
    })
}

/// Analyze a MOVE statement
///
/// Validates the node references (path format, ID format)
/// and resolves the table name to a workspace.
pub(super) fn analyze_move(catalog: &dyn Catalog, stmt: &MoveStatement) -> Result<AnalyzedMove> {
    // Resolve table name to workspace
    let workspace = catalog
        .resolve_workspace_name(&stmt.table)
        .ok_or_else(|| AnalysisError::TableNotFound(stmt.table.clone()))?;

    // Validate source reference
    validate_node_reference(&stmt.source)?;

    // Validate target parent reference
    validate_node_reference(&stmt.target_parent)?;

    // Check for self-reference (cannot move into self)
    // This is a basic static check - deeper validation happens at execution
    match (&stmt.source, &stmt.target_parent) {
        (NodeReference::Path(s), NodeReference::Path(t)) if s == t => {
            return Err(AnalysisError::MoveSelfReference);
        }
        (NodeReference::Id(s), NodeReference::Id(t)) if s == t => {
            return Err(AnalysisError::MoveSelfReference);
        }
        _ => {
            // Mixed path/id - self-reference check happens at execution time
        }
    }

    // Check for circular reference (cannot move into descendant)
    // This can only be done statically when both are path references
    if let (NodeReference::Path(source_path), NodeReference::Path(target_path)) =
        (&stmt.source, &stmt.target_parent)
    {
        if target_path.starts_with(&format!("{}/", source_path)) {
            return Err(AnalysisError::MoveCircularReference {
                source_path: source_path.clone(),
                target_path: target_path.clone(),
            });
        }
    }

    Ok(AnalyzedMove {
        table: stmt.table.clone(),
        workspace,
        source: stmt.source.clone(),
        target_parent: stmt.target_parent.clone(),
        branch_override: stmt.branch.clone(),
    })
}

/// Analyze a COPY statement
///
/// Validates the node references (path format, ID format)
/// and resolves the table name to a workspace.
pub(super) fn analyze_copy(catalog: &dyn Catalog, stmt: &CopyStatement) -> Result<AnalyzedCopy> {
    // Resolve table name to workspace
    let workspace = catalog
        .resolve_workspace_name(&stmt.table)
        .ok_or_else(|| AnalysisError::TableNotFound(stmt.table.clone()))?;

    // Validate source reference
    validate_node_reference(&stmt.source)?;

    // Validate target parent reference
    validate_node_reference(&stmt.target_parent)?;

    // Check for self-reference (cannot copy into self)
    // This is a basic static check - deeper validation happens at execution
    match (&stmt.source, &stmt.target_parent) {
        (NodeReference::Path(s), NodeReference::Path(t)) if s == t => {
            return Err(AnalysisError::CopySelfReference);
        }
        (NodeReference::Id(s), NodeReference::Id(t)) if s == t => {
            return Err(AnalysisError::CopySelfReference);
        }
        _ => {
            // Mixed path/id - self-reference check happens at execution time
        }
    }

    // Check for circular reference (cannot copy into descendant)
    // This can only be done statically when both are path references
    if let (NodeReference::Path(source_path), NodeReference::Path(target_path)) =
        (&stmt.source, &stmt.target_parent)
    {
        if target_path.starts_with(&format!("{}/", source_path)) {
            return Err(AnalysisError::CopyCircularReference {
                source_path: source_path.clone(),
                target_path: target_path.clone(),
            });
        }
    }

    Ok(AnalyzedCopy {
        table: stmt.table.clone(),
        workspace,
        source: stmt.source.clone(),
        target_parent: stmt.target_parent.clone(),
        new_name: stmt.new_name.clone(),
        recursive: stmt.recursive,
        branch_override: stmt.branch.clone(),
    })
}

/// Analyze a RESTORE statement
///
/// Validates the node reference (path format, ID format).
/// The revision reference is kept as-is and resolved at execution time
/// when branch context is available.
pub(super) fn analyze_restore(stmt: &RestoreStatement) -> Result<AnalyzedRestore> {
    // Validate node reference
    validate_node_reference(&stmt.node)?;

    // Validate translations if specified
    if let Some(translations) = &stmt.translations {
        for locale in translations {
            if locale.is_empty() {
                return Err(AnalysisError::TranslateEmptyLocale);
            }
            if !locale
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                return Err(AnalysisError::TranslateInvalidLocale(locale.clone()));
            }
        }
    }

    Ok(AnalyzedRestore {
        node: stmt.node.clone(),
        revision: stmt.revision.clone(),
        recursive: stmt.recursive,
        translations: stmt.translations.clone(),
        branch_override: None, // RESTORE doesn't currently support IN BRANCH clause
    })
}

/// Analyze a TRANSLATE statement
///
/// Validates the locale, converts paths to JsonPointers, and resolves the table
/// name to a workspace.
pub(super) fn analyze_translate(
    catalog: &dyn Catalog,
    stmt: &TranslateStatement,
) -> Result<AnalyzedTranslate> {
    // Resolve table name to workspace
    let workspace = catalog
        .resolve_workspace_name(&stmt.table)
        .ok_or_else(|| AnalysisError::TableNotFound(stmt.table.clone()))?;

    // Validate locale code (basic validation - alphanumeric with dashes)
    if stmt.locale.is_empty() {
        return Err(AnalysisError::TranslateEmptyLocale);
    }
    if !stmt
        .locale
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AnalysisError::TranslateInvalidLocale(stmt.locale.clone()));
    }

    // Convert assignments to analyzed format
    let mut node_translations: HashMap<String, AnalyzedTranslationValue> = HashMap::new();
    let mut block_translations: HashMap<String, HashMap<String, AnalyzedTranslationValue>> =
        HashMap::new();

    for assignment in &stmt.assignments {
        let value = match &assignment.value {
            TranslationValue::String(s) => AnalyzedTranslationValue::String(s.clone()),
            TranslationValue::Integer(i) => AnalyzedTranslationValue::Integer(*i),
            TranslationValue::Float(f) => AnalyzedTranslationValue::Float(*f),
            TranslationValue::Boolean(b) => AnalyzedTranslationValue::Boolean(*b),
            TranslationValue::Null => AnalyzedTranslationValue::Null,
        };

        match &assignment.path {
            TranslationPath::Property(segments) => {
                // Convert dot-notation to JsonPointer
                let json_pointer = format!("/{}", segments.join("/"));
                node_translations.insert(json_pointer, value);
            }
            TranslationPath::BlockProperty {
                block_uuid,
                property_path,
                ..
            } => {
                // Validate block UUID format
                if block_uuid.is_empty() {
                    return Err(AnalysisError::TranslateEmptyBlockUuid);
                }

                // Convert property path to JsonPointer
                let json_pointer = format!("/{}", property_path.join("/"));

                // Add to block translations map
                block_translations
                    .entry(block_uuid.clone())
                    .or_default()
                    .insert(json_pointer, value);
            }
        }
    }

    // Convert filter
    let filter = stmt.filter.as_ref().map(|f| match f {
        TranslateFilter::Path(p) => AnalyzedTranslateFilter::Path(p.clone()),
        TranslateFilter::Id(i) => AnalyzedTranslateFilter::Id(i.clone()),
        TranslateFilter::PathAndType { path, node_type } => AnalyzedTranslateFilter::PathAndType {
            path: path.clone(),
            node_type: node_type.clone(),
        },
        TranslateFilter::IdAndType { id, node_type } => AnalyzedTranslateFilter::IdAndType {
            id: id.clone(),
            node_type: node_type.clone(),
        },
        TranslateFilter::NodeType(nt) => AnalyzedTranslateFilter::NodeType(nt.clone()),
    });

    Ok(AnalyzedTranslate {
        table: stmt.table.clone(),
        workspace,
        locale: stmt.locale.clone(),
        node_translations,
        block_translations,
        filter,
        branch_override: stmt.branch.clone(),
    })
}

/// Analyze a RELATE statement
///
/// Validates the node references and resolves workspaces.
pub(super) fn analyze_relate(stmt: &RelateStatement) -> Result<AnalyzedRelate> {
    // Validate source reference
    validate_relate_node_reference(&stmt.source.node_ref)?;

    // Validate target reference
    validate_relate_node_reference(&stmt.target.node_ref)?;

    // Check for self-reference (same path or same ID)
    match (&stmt.source.node_ref, &stmt.target.node_ref) {
        (RelateNodeReference::Path(s), RelateNodeReference::Path(t)) if s == t => {
            return Err(AnalysisError::RelateSelfReference);
        }
        (RelateNodeReference::Id(s), RelateNodeReference::Id(t)) if s == t => {
            return Err(AnalysisError::RelateSelfReference);
        }
        _ => {
            // Mixed path/id - self-reference check happens at execution time
        }
    }

    // Resolve source workspace (uses "default" if not specified)
    let source_workspace = stmt
        .source
        .workspace
        .clone()
        .unwrap_or_else(|| "default".to_string());

    // Resolve target workspace (uses "default" if not specified)
    let target_workspace = stmt
        .target
        .workspace
        .clone()
        .unwrap_or_else(|| "default".to_string());

    // Default relation type is "references"
    let relation_type = stmt
        .relation_type
        .clone()
        .unwrap_or_else(|| "references".to_string());

    Ok(AnalyzedRelate {
        source: AnalyzedRelateEndpoint {
            node_ref: stmt.source.node_ref.clone(),
            workspace: source_workspace,
        },
        target: AnalyzedRelateEndpoint {
            node_ref: stmt.target.node_ref.clone(),
            workspace: target_workspace,
        },
        relation_type,
        weight: stmt.weight,
        branch_override: stmt.branch.clone(),
    })
}

/// Analyze an UNRELATE statement
///
/// Validates the node references and resolves workspaces.
pub(super) fn analyze_unrelate(stmt: &UnrelateStatement) -> Result<AnalyzedUnrelate> {
    // Validate source reference
    validate_relate_node_reference(&stmt.source.node_ref)?;

    // Validate target reference
    validate_relate_node_reference(&stmt.target.node_ref)?;

    // Check for self-reference (same path or same ID)
    match (&stmt.source.node_ref, &stmt.target.node_ref) {
        (RelateNodeReference::Path(s), RelateNodeReference::Path(t)) if s == t => {
            return Err(AnalysisError::RelateSelfReference);
        }
        (RelateNodeReference::Id(s), RelateNodeReference::Id(t)) if s == t => {
            return Err(AnalysisError::RelateSelfReference);
        }
        _ => {
            // Mixed path/id - self-reference check happens at execution time
        }
    }

    // Resolve source workspace (uses "default" if not specified)
    let source_workspace = stmt
        .source
        .workspace
        .clone()
        .unwrap_or_else(|| "default".to_string());

    // Resolve target workspace (uses "default" if not specified)
    let target_workspace = stmt
        .target
        .workspace
        .clone()
        .unwrap_or_else(|| "default".to_string());

    Ok(AnalyzedUnrelate {
        source: AnalyzedRelateEndpoint {
            node_ref: stmt.source.node_ref.clone(),
            workspace: source_workspace,
        },
        target: AnalyzedRelateEndpoint {
            node_ref: stmt.target.node_ref.clone(),
            workspace: target_workspace,
        },
        relation_type: stmt.relation_type.clone(),
        branch_override: stmt.branch.clone(),
    })
}

/// Validate a node reference (path or ID)
fn validate_node_reference(node_ref: &NodeReference) -> Result<()> {
    match node_ref {
        NodeReference::Path(path) => {
            if path.is_empty() {
                return Err(AnalysisError::OrderEmptyPath);
            }
            if !path.starts_with('/') {
                return Err(AnalysisError::InvalidPath(format!(
                    "Path must start with '/': {}",
                    path
                )));
            }
            if path == "/" {
                return Err(AnalysisError::OrderRootNodeNotAllowed);
            }
            Ok(())
        }
        NodeReference::Id(id) => {
            if id.is_empty() {
                return Err(AnalysisError::OrderEmptyId);
            }
            // Basic ID format validation - alphanumeric, dash, underscore
            if !id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                return Err(AnalysisError::OrderInvalidId(format!(
                    "ID contains invalid characters: {}",
                    id
                )));
            }
            Ok(())
        }
    }
}

/// Validate a RELATE/UNRELATE node reference (path or ID)
fn validate_relate_node_reference(node_ref: &RelateNodeReference) -> Result<()> {
    match node_ref {
        RelateNodeReference::Path(path) => {
            if path.is_empty() {
                return Err(AnalysisError::RelateEmptyPath);
            }
            if !path.starts_with('/') {
                return Err(AnalysisError::InvalidPath(format!(
                    "Path must start with '/': {}",
                    path
                )));
            }
            Ok(())
        }
        RelateNodeReference::Id(id) => {
            if id.is_empty() {
                return Err(AnalysisError::RelateEmptyId);
            }
            // Basic ID format validation - alphanumeric, dash, underscore
            if !id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                return Err(AnalysisError::RelateInvalidId(format!(
                    "ID contains invalid characters: {}",
                    id
                )));
            }
            Ok(())
        }
    }
}

/// Extract the parent path from a full path
///
/// Examples:
/// - "/content/page1" -> "/content"
/// - "/blog" -> "/"
/// - "/" -> "/" (root has no parent, but we return root for consistency)
fn get_parent_path(path: &str) -> &str {
    if path == "/" {
        return "/";
    }

    // Find the last '/' and return everything before it
    match path.rfind('/') {
        Some(0) => "/", // Path like "/blog" -> parent is "/"
        Some(pos) => &path[..pos],
        None => "/", // Shouldn't happen for valid paths starting with '/'
    }
}
