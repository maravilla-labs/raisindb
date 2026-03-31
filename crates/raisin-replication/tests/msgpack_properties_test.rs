use raisin_models::nodes::properties::PropertyValue;
use raisin_replication::OpType;
use std::collections::HashMap;

#[test]
fn test_create_node_with_properties_msgpack_roundtrip() {
    let mut props = HashMap::new();
    props.insert(
        "content".to_string(),
        PropertyValue::String("Hello World".to_string()),
    );

    let op_type = OpType::CreateNode {
        node_id: "article-1".to_string(),
        name: "My First Article".to_string(),
        node_type: "Article".to_string(),
        archetype: None,
        parent_id: None,
        order_key: "a".to_string(),
        properties: props.clone(),
        owner_id: None,
        workspace: None,
        path: "/My First Article".to_string(),
    };

    // Serialize to MessagePack
    let bytes = rmp_serde::to_vec(&op_type).unwrap();
    println!("Serialized {} bytes", bytes.len());
    println!("First 100 bytes: {:?}", &bytes[..bytes.len().min(100)]);

    // Deserialize
    let roundtrip: OpType = rmp_serde::from_slice(&bytes).unwrap();
    println!("Deserialized successfully!");

    if let OpType::CreateNode {
        properties: rt_props,
        ..
    } = roundtrip
    {
        assert_eq!(rt_props.len(), 1);
        assert_eq!(
            rt_props.get("content"),
            Some(&PropertyValue::String("Hello World".to_string()))
        );
    } else {
        panic!("Expected CreateNode variant");
    }
}
