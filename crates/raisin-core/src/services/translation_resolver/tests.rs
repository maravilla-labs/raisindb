use super::*;
use raisin_models::nodes::properties::value::Element;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage_memory::NoopTranslationRepo;
use std::collections::HashMap;

#[test]
fn test_merge_property_simple() {
    let resolver = create_test_resolver();
    let mut node = create_test_node();

    let pointer = JsonPointer::new("/title");
    let value = PropertyValue::String("Translated Title".to_string());

    resolver.merge_property(&mut node, &pointer, value).unwrap();

    assert_eq!(
        node.properties.get("title"),
        Some(&PropertyValue::String("Translated Title".to_string()))
    );
}

#[test]
fn test_merge_property_nested() {
    let resolver = create_test_resolver();
    let mut node = create_test_node();

    let mut metadata = HashMap::new();
    metadata.insert(
        "author".to_string(),
        PropertyValue::String("Original Author".to_string()),
    );
    node.properties
        .insert("metadata".to_string(), PropertyValue::Object(metadata));

    let pointer = JsonPointer::new("/metadata/description");
    let value = PropertyValue::String("Translated Description".to_string());

    resolver.merge_property(&mut node, &pointer, value).unwrap();

    if let Some(PropertyValue::Object(meta)) = node.properties.get("metadata") {
        assert_eq!(
            meta.get("description"),
            Some(&PropertyValue::String("Translated Description".to_string()))
        );
        assert_eq!(
            meta.get("author"),
            Some(&PropertyValue::String("Original Author".to_string()))
        );
    } else {
        panic!("Expected metadata to be an object");
    }
}

#[test]
fn test_merge_property_creates_intermediate_objects() {
    let resolver = create_test_resolver();
    let mut node = create_test_node();

    let pointer = JsonPointer::new("/seo/meta/description");
    let value = PropertyValue::String("SEO Description".to_string());

    resolver.merge_property(&mut node, &pointer, value).unwrap();

    if let Some(PropertyValue::Object(seo)) = node.properties.get("seo") {
        if let Some(PropertyValue::Object(meta)) = seo.get("meta") {
            assert_eq!(
                meta.get("description"),
                Some(&PropertyValue::String("SEO Description".to_string()))
            );
        } else {
            panic!("Expected seo.meta to be an object");
        }
    } else {
        panic!("Expected seo to be an object");
    }
}

#[test]
fn test_find_block_containers() {
    let resolver = create_test_resolver();

    let mut properties = HashMap::new();

    let mut block1 = HashMap::new();
    block1.insert(
        "uuid".to_string(),
        PropertyValue::String("block-uuid-1".to_string()),
    );
    block1.insert(
        "type".to_string(),
        PropertyValue::String("text".to_string()),
    );

    let blocks = vec![PropertyValue::Object(block1)];
    properties.insert("content".to_string(), PropertyValue::Array(blocks));

    let regular_array = vec![PropertyValue::String("item1".to_string())];
    properties.insert("tags".to_string(), PropertyValue::Array(regular_array));

    let paths = resolver.find_block_containers(&properties);

    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], "content");
}

#[test]
fn test_merge_property_through_array_by_uuid() {
    let resolver = create_test_resolver();
    let mut node = create_test_node();

    // Build a section array with UUID-keyed objects
    let mut block = HashMap::new();
    block.insert(
        "uuid".to_string(),
        PropertyValue::String("hero-1".to_string()),
    );
    block.insert(
        "headline".to_string(),
        PropertyValue::String("Original".to_string()),
    );
    node.properties.insert(
        "content".to_string(),
        PropertyValue::Array(vec![PropertyValue::Object(block)]),
    );

    // Merge a translation targeting /content/hero-1/headline
    let pointer = JsonPointer::new("/content/hero-1/headline");
    let value = PropertyValue::String("Überschrift".to_string());
    resolver.merge_property(&mut node, &pointer, value).unwrap();

    // Verify the array element was updated
    if let Some(PropertyValue::Array(arr)) = node.properties.get("content") {
        if let Some(PropertyValue::Object(obj)) = arr.first() {
            assert_eq!(
                obj.get("headline"),
                Some(&PropertyValue::String("Überschrift".to_string()))
            );
        } else {
            panic!("Expected first array element to be an object");
        }
    } else {
        panic!("Expected content to be an array");
    }
}

#[test]
fn test_merge_property_array_uuid_not_found() {
    let resolver = create_test_resolver();
    let mut node = create_test_node();

    let mut block = HashMap::new();
    block.insert(
        "uuid".to_string(),
        PropertyValue::String("hero-1".to_string()),
    );
    block.insert(
        "headline".to_string(),
        PropertyValue::String("Original".to_string()),
    );
    node.properties.insert(
        "content".to_string(),
        PropertyValue::Array(vec![PropertyValue::Object(block)]),
    );

    // Target a UUID that doesn't exist — should silently skip
    let pointer = JsonPointer::new("/content/nonexistent-uuid/headline");
    let value = PropertyValue::String("Nope".to_string());
    resolver.merge_property(&mut node, &pointer, value).unwrap();

    // Original value unchanged
    if let Some(PropertyValue::Array(arr)) = node.properties.get("content") {
        if let Some(PropertyValue::Object(obj)) = arr.first() {
            assert_eq!(
                obj.get("headline"),
                Some(&PropertyValue::String("Original".to_string()))
            );
        } else {
            panic!("Expected first array element to be an object");
        }
    } else {
        panic!("Expected content to be an array");
    }
}

#[test]
fn test_merge_property_nested_within_array_element() {
    let resolver = create_test_resolver();
    let mut node = create_test_node();

    // Build block with nested object
    let mut inner = HashMap::new();
    inner.insert(
        "title".to_string(),
        PropertyValue::String("Old".to_string()),
    );
    let mut block = HashMap::new();
    block.insert(
        "uuid".to_string(),
        PropertyValue::String("card-1".to_string()),
    );
    block.insert("meta".to_string(), PropertyValue::Object(inner));

    node.properties.insert(
        "content".to_string(),
        PropertyValue::Array(vec![PropertyValue::Object(block)]),
    );

    // Merge into nested path /content/card-1/meta/title
    let pointer = JsonPointer::new("/content/card-1/meta/title");
    let value = PropertyValue::String("Neu".to_string());
    resolver.merge_property(&mut node, &pointer, value).unwrap();

    if let Some(PropertyValue::Array(arr)) = node.properties.get("content") {
        if let Some(PropertyValue::Object(obj)) = arr.first() {
            if let Some(PropertyValue::Object(meta)) = obj.get("meta") {
                assert_eq!(
                    meta.get("title"),
                    Some(&PropertyValue::String("Neu".to_string()))
                );
            } else {
                panic!("Expected meta to be an object");
            }
        } else {
            panic!("Expected first array element to be an object");
        }
    } else {
        panic!("Expected content to be an array");
    }
}

#[test]
fn test_merge_property_through_element_by_uuid() {
    let resolver = create_test_resolver();
    let mut node = create_test_node();

    // Build a content array with Element items (like Hero, TextBlock, FeatureGrid)
    let mut hero_content = HashMap::new();
    hero_content.insert(
        "headline".to_string(),
        PropertyValue::String("Welcome".to_string()),
    );
    hero_content.insert(
        "subheadline".to_string(),
        PropertyValue::String("Original subtitle".to_string()),
    );

    let hero = Element {
        uuid: "hero-1".to_string(),
        element_type: "launchpad:Hero".to_string(),
        content: hero_content,
    };

    let mut text_content = HashMap::new();
    text_content.insert(
        "body".to_string(),
        PropertyValue::String("Original body".to_string()),
    );

    let text_block = Element {
        uuid: "text-1".to_string(),
        element_type: "launchpad:TextBlock".to_string(),
        content: text_content,
    };

    node.properties.insert(
        "content".to_string(),
        PropertyValue::Array(vec![
            PropertyValue::Element(hero),
            PropertyValue::Element(text_block),
        ]),
    );

    // Merge a translation targeting /content/hero-1/headline
    let pointer = JsonPointer::new("/content/hero-1/headline");
    let value = PropertyValue::String("Willkommen".to_string());
    resolver.merge_property(&mut node, &pointer, value).unwrap();

    // Merge a translation targeting /content/text-1/body
    let pointer2 = JsonPointer::new("/content/text-1/body");
    let value2 = PropertyValue::String("Übersetzter Text".to_string());
    resolver
        .merge_property(&mut node, &pointer2, value2)
        .unwrap();

    // Verify the Element items were updated
    if let Some(PropertyValue::Array(arr)) = node.properties.get("content") {
        if let Some(PropertyValue::Element(hero)) = arr.first() {
            assert_eq!(hero.uuid, "hero-1");
            assert_eq!(
                hero.content.get("headline"),
                Some(&PropertyValue::String("Willkommen".to_string()))
            );
            // subheadline should be untouched
            assert_eq!(
                hero.content.get("subheadline"),
                Some(&PropertyValue::String("Original subtitle".to_string()))
            );
        } else {
            panic!("Expected first array element to be an Element");
        }

        if let Some(PropertyValue::Element(text)) = arr.get(1) {
            assert_eq!(text.uuid, "text-1");
            assert_eq!(
                text.content.get("body"),
                Some(&PropertyValue::String("Übersetzter Text".to_string()))
            );
        } else {
            panic!("Expected second array element to be an Element");
        }
    } else {
        panic!("Expected content to be an array");
    }
}

#[test]
fn test_find_block_containers_with_elements() {
    let resolver = create_test_resolver();

    let mut properties = HashMap::new();

    let element = Element {
        uuid: "hero-1".to_string(),
        element_type: "launchpad:Hero".to_string(),
        content: HashMap::new(),
    };

    properties.insert(
        "content".to_string(),
        PropertyValue::Array(vec![PropertyValue::Element(element)]),
    );

    let paths = resolver.find_block_containers(&properties);

    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], "content");
}

// Helper functions

fn create_test_resolver() -> TranslationResolver<NoopTranslationRepo> {
    let repo = Arc::new(NoopTranslationRepo::default());
    let config = RepositoryConfig {
        locale_fallback_chains: HashMap::new(),
        ..Default::default()
    };
    TranslationResolver::new(repo, config)
}

fn create_test_node() -> Node {
    Node {
        id: "test-node".to_string(),
        name: "Test Node".to_string(),
        path: "/test".to_string(),
        node_type: "raisin:page".to_string(),
        archetype: None,
        properties: HashMap::new(),
        children: vec![],
        order_key: "a".to_string(),
        has_children: None,
        parent: None,
        version: 1,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        created_by: None,
        translations: None,
        tenant_id: None,
        workspace: None,
        owner_id: None,
        relations: Vec::new(),
    }
}
