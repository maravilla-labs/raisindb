//! Binary file helpers for package creation
//!
//! Extracts storage keys from node properties for embedded file retrieval.

use super::PackageCreateFromSelectionHandler;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;

impl PackageCreateFromSelectionHandler {
    /// Get the storage key for an embedded file from a node's properties
    ///
    /// Checks for file/resource properties in these formats:
    /// 1. PropertyValue::Resource with metadata.storage_key (standard upload format)
    /// 2. PropertyValue::Object with key field (package format)
    pub(super) fn get_embedded_file_storage_key(&self, node: &Node) -> Option<String> {
        // Check for "file" property as PropertyValue::Resource (standard upload format)
        if let Some(PropertyValue::Resource(resource)) = node.properties.get("file") {
            if let Some(ref metadata) = resource.metadata {
                if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                    return Some(key.clone());
                }
            }
        }

        // Check for "resource" property as PropertyValue::Resource
        if let Some(PropertyValue::Resource(resource)) = node.properties.get("resource") {
            if let Some(ref metadata) = resource.metadata {
                if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                    return Some(key.clone());
                }
            }
        }

        // Also check for Object format (used by packages and some legacy code)
        if let Some(PropertyValue::Object(file_obj)) = node.properties.get("file") {
            // Check for metadata.storage_key
            if let Some(PropertyValue::Object(metadata)) = file_obj.get("metadata") {
                if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                    return Some(key.clone());
                }
            }
            // Check for direct key field
            if let Some(PropertyValue::String(key)) = file_obj.get("key") {
                return Some(key.clone());
            }
        }

        // Check for "resource" property as Object
        if let Some(PropertyValue::Object(resource_obj)) = node.properties.get("resource") {
            if let Some(PropertyValue::Object(metadata)) = resource_obj.get("metadata") {
                if let Some(PropertyValue::String(key)) = metadata.get("storage_key") {
                    return Some(key.clone());
                }
            }
            // Also check for direct key in resource
            if let Some(PropertyValue::String(key)) = resource_obj.get("key") {
                return Some(key.clone());
            }
        }

        None
    }
}
