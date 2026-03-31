// SPDX-License-Identifier: BSL-1.1

//! Build resource property values from stored objects.

use crate::upload_processors::StorageFormat;

/// Build a resource property value based on the storage format.
pub(super) fn build_resource_value(
    stored: &raisin_binary::StoredObject,
    storage_format: StorageFormat,
) -> raisin_models::nodes::properties::PropertyValue {
    match storage_format {
        StorageFormat::Resource => {
            let mut metadata = std::collections::HashMap::new();
            metadata.insert(
                "storage_key".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(stored.key.clone()),
            );

            let resource = raisin_models::nodes::properties::value::Resource {
                uuid: nanoid::nanoid!(),
                name: stored.name.clone(),
                size: Some(stored.size),
                mime_type: stored.mime_type.clone(),
                url: Some(stored.key.clone()),
                metadata: Some(metadata),
                is_loaded: Some(true),
                is_external: Some(false),
                created_at: stored.created_at.into(),
                updated_at: stored.updated_at.into(),
            };
            raisin_models::nodes::properties::PropertyValue::Resource(resource)
        }
        StorageFormat::Object => {
            let mut resource_obj = std::collections::HashMap::new();
            resource_obj.insert(
                "key".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(stored.key.clone()),
            );
            resource_obj.insert(
                "url".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(stored.url.clone()),
            );
            if let Some(mime) = &stored.mime_type {
                resource_obj.insert(
                    "mime_type".to_string(),
                    raisin_models::nodes::properties::PropertyValue::String(mime.clone()),
                );
            }
            resource_obj.insert(
                "size".to_string(),
                raisin_models::nodes::properties::PropertyValue::Integer(stored.size),
            );
            raisin_models::nodes::properties::PropertyValue::Object(resource_obj)
        }
    }
}
