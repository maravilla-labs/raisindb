//! The JSON surface of the resolver: documents that only exist as JSON, and
//! the JSON rendering of a node. The engine itself walks stored values
//! (`doc.rs`, plan Phase 13d).

use super::{doc, ReferenceResolver};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::Storage;

impl<S: Storage> ReferenceResolver<S> {
    /// Resolve every reference inside a JSON value.
    ///
    /// - a reference is any object carrying a string `raisin:ref` (an id, or a
    ///   path when it starts with `/`); its `raisin:workspace` defaults to
    ///   `workspace` when absent or empty — also for references found inside
    ///   inlined targets;
    /// - `max_depth` (capped at [`MAX_RESOLUTION_DEPTH`]) bounds how deep
    ///   inlined nodes nest, which is also what makes a cycle terminate;
    /// - a reference that cannot be resolved — missing, hidden in the locale,
    ///   or not readable by the caller — is kept as it was written;
    /// - `fields`, when given, trims every inlined node to `id`, `name`,
    ///   `path`, `node_type` plus the listed properties, and only references
    ///   inside what is kept are followed.
    ///
    /// The value may itself be a single reference, which resolves to the node.
    /// This is [`Self::resolve_values_many`] over the JSON's structure (a
    /// number beyond `i64` reads back as a float).
    pub async fn resolve_json(
        &self,
        workspace: &str,
        value: &serde_json::Value,
        max_depth: u32,
        fields: Option<&[String]>,
    ) -> Result<serde_json::Value> {
        let mut out = self
            .resolve_json_many(workspace, &[value], max_depth, fields)
            .await?;
        Ok(out.pop().unwrap_or_else(|| value.clone()))
    }

    /// [`Self::resolve_json`] for several documents at once.
    pub async fn resolve_json_many(
        &self,
        workspace: &str,
        values: &[&serde_json::Value],
        max_depth: u32,
        fields: Option<&[String]>,
    ) -> Result<Vec<serde_json::Value>> {
        let docs = values
            .iter()
            .map(|v| doc::structural((*v).clone()))
            .collect();
        let resolved = self
            .resolve_values_many(workspace, docs, max_depth, fields)
            .await?;
        Ok(resolved
            .iter()
            .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
            .collect())
    }
}

/// A JSON document as the value tree RESOLVE walks: the structure as it is,
/// nothing classified, so it renders back to itself (a number beyond `i64`
/// excepted). For a document that only exists as JSON — an expression's
/// result rather than a stored value.
pub fn document_from_json(
    json: serde_json::Value,
) -> raisin_models::nodes::properties::PropertyValue {
    doc::structural(json)
}

/// Convert a Node to a `serde_json::Value` for RESOLVE() SQL function output
///
/// Returns an object with: id, name, path, node_type, plus all properties flattened.
pub fn node_to_json_value(node: &Node) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), serde_json::Value::String(node.id.clone()));
    map.insert(
        "name".to_string(),
        serde_json::Value::String(node.name.clone()),
    );
    map.insert(
        "path".to_string(),
        serde_json::Value::String(node.path.clone()),
    );
    map.insert(
        "node_type".to_string(),
        serde_json::Value::String(node.node_type.clone()),
    );

    // Flatten properties into the object
    if let Ok(serde_json::Value::Object(props_map)) = serde_json::to_value(&node.properties) {
        for (k, v) in props_map {
            map.insert(k, v);
        }
    }

    serde_json::Value::Object(map)
}

/// [`node_to_json_value`], optionally keeping only `fields` of the properties.
///
/// The identity members (`id`, `name`, `path`, `node_type`) are always kept:
/// they are what a renderer links and keys by, and a trimmed node without them
/// could not be told apart from its neighbours.
pub fn node_to_json_value_with_fields(node: &Node, fields: Option<&[String]>) -> serde_json::Value {
    let Some(fields) = fields else {
        return node_to_json_value(node);
    };
    let mut map = serde_json::Map::with_capacity(4 + fields.len());
    map.insert("id".into(), serde_json::Value::String(node.id.clone()));
    map.insert("name".into(), serde_json::Value::String(node.name.clone()));
    map.insert("path".into(), serde_json::Value::String(node.path.clone()));
    map.insert(
        "node_type".into(),
        serde_json::Value::String(node.node_type.clone()),
    );
    for field in fields {
        if let Some(value) = node.properties.get(field) {
            if let Ok(json) = serde_json::to_value(value) {
                map.insert(field.clone(), json);
            }
        }
    }
    serde_json::Value::Object(map)
}
