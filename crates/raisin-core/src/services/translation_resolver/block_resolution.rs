//! Block-level translation resolution for Composite properties.
//!
//! Blocks are identified by their stable UUID. Each block translation
//! is stored separately and can translate any property within the block.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay};
use raisin_storage::TranslationRepository;
use std::collections::HashMap;

use super::TranslationResolver;

impl<R: TranslationRepository> TranslationResolver<R> {
    /// Resolve block-level translations for Composite properties.
    ///
    /// Scans the node's properties for Composite types and fetches
    /// translations for each block by UUID, applying them to the block's properties.
    pub(super) async fn resolve_block_translations(
        &self,
        node: &mut Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        locale: &LocaleCode,
        revision: &raisin_hlc::HLC,
    ) -> Result<()> {
        let mut blocks_to_translate = Vec::new();

        let mut stack: Vec<&HashMap<String, PropertyValue>> = vec![&node.properties];

        while let Some(properties) = stack.pop() {
            for (_key, value) in properties.iter() {
                match value {
                    PropertyValue::Array(items) => {
                        for item in items.iter() {
                            match item {
                                PropertyValue::Object(obj) => {
                                    if let Some(PropertyValue::String(uuid)) = obj.get("uuid") {
                                        blocks_to_translate.push(uuid.clone());
                                    }
                                    stack.push(obj);
                                }
                                PropertyValue::Element(element) => {
                                    blocks_to_translate.push(element.uuid.clone());
                                    stack.push(&element.content);
                                }
                                _ => {}
                            }
                        }
                    }
                    PropertyValue::Object(obj) => {
                        stack.push(obj);
                    }
                    _ => {}
                }
            }
        }

        for uuid in blocks_to_translate {
            let block_overlay = self
                .repository
                .get_block_translation(
                    tenant_id, repo_id, branch, workspace, &node.id, &uuid, locale, revision,
                )
                .await?;

            if let Some(LocaleOverlay::Properties { data }) = block_overlay {
                self.apply_block_translation_by_uuid(&mut node.properties, &uuid, data)?;
            }
        }

        Ok(())
    }

    /// Apply a block translation to a specific block identified by UUID.
    pub(super) fn apply_block_translation_by_uuid(
        &self,
        properties: &mut HashMap<String, PropertyValue>,
        target_uuid: &str,
        translations: HashMap<JsonPointer, PropertyValue>,
    ) -> Result<()> {
        let mut stack: Vec<&mut PropertyValue> = properties.values_mut().collect();

        while let Some(value) = stack.pop() {
            match value {
                PropertyValue::Array(items) => {
                    for item in items.iter_mut() {
                        match item {
                            PropertyValue::Object(obj) => {
                                if let Some(PropertyValue::String(uuid)) = obj.get("uuid") {
                                    if uuid == target_uuid {
                                        for (pointer, translation_value) in &translations {
                                            self.merge_block_property(
                                                obj,
                                                pointer,
                                                translation_value.clone(),
                                            )?;
                                        }
                                        return Ok(());
                                    }
                                }
                                stack.extend(obj.values_mut());
                            }
                            PropertyValue::Element(element) => {
                                if element.uuid == target_uuid {
                                    for (pointer, translation_value) in &translations {
                                        self.merge_block_property(
                                            &mut element.content,
                                            pointer,
                                            translation_value.clone(),
                                        )?;
                                    }
                                    return Ok(());
                                }
                                stack.extend(element.content.values_mut());
                            }
                            other => {
                                stack.push(other);
                            }
                        }
                    }
                }
                PropertyValue::Object(obj) => {
                    stack.extend(obj.values_mut());
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Find all property paths that contain Composite arrays.
    pub(super) fn find_block_containers(
        &self,
        properties: &HashMap<String, PropertyValue>,
    ) -> Vec<String> {
        let mut paths = Vec::new();
        find_block_containers_recursive(properties, "", &mut paths);
        paths
    }

    /// Get a reference to a property at the given path.
    pub(super) fn get_property_at_path<'a>(
        &self,
        node: &'a Node,
        path: &str,
    ) -> Option<&'a PropertyValue> {
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut current = &node.properties;

        for segment in &segments[..segments.len() - 1] {
            match current.get(*segment) {
                Some(PropertyValue::Object(obj)) => {
                    current = obj;
                }
                _ => return None,
            }
        }

        current.get(segments[segments.len() - 1])
    }

    /// Merge a property value into a block object using a JsonPointer path.
    pub(super) fn merge_block_property(
        &self,
        block: &mut HashMap<String, PropertyValue>,
        pointer: &JsonPointer,
        value: PropertyValue,
    ) -> Result<()> {
        let segments = pointer.segments();
        if segments.is_empty() {
            return Err(Error::Validation(
                "Cannot merge empty JsonPointer path".to_string(),
            ));
        }
        super::merge_into_map(block, &segments, value)
    }
}

fn find_block_containers_recursive(
    properties: &HashMap<String, PropertyValue>,
    current_path: &str,
    results: &mut Vec<String>,
) {
    for (key, value) in properties {
        let path = if current_path.is_empty() {
            key.clone()
        } else {
            format!("{}/{}", current_path, key)
        };

        match value {
            PropertyValue::Array(arr) => {
                if arr.iter().any(|item| match item {
                    PropertyValue::Object(obj) => obj.contains_key("uuid"),
                    PropertyValue::Element(_) => true,
                    _ => false,
                }) {
                    results.push(path);
                }
            }
            PropertyValue::Object(obj) => {
                find_block_containers_recursive(obj, &path, results);
            }
            _ => {}
        }
    }
}
