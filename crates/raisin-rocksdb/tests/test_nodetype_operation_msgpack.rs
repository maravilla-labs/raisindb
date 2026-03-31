use raisin_models::nodes::element::element_type::ElementType;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_replication::{OpType, Operation, VectorClock};

#[test]
fn test_nodetype_in_operation_msgpack() {
    let node_type = NodeType {
        id: Some("test123".to_string()),
        strict: None,
        name: "Article".to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: Some("Article node type".to_string()),
        icon: None,
        version: None,
        properties: None,
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: None,
        publishable: None,
        auditable: None,
        indexable: None,
        index_types: None,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    };

    let mut vc = VectorClock::new();
    vc.increment("node1");

    let op = Operation::new(
        1,
        "node1".to_string(),
        vc,
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        OpType::UpdateNodeType {
            node_type_id: "Article".to_string(),
            node_type,
        },
        "test_actor".to_string(),
    );

    println!("Original operation: {:?}", op);

    // Test COMPACT format (to_vec) - this is what OpLog uses!
    let bytes_compact = rmp_serde::to_vec(&op).expect("Compact serialize failed");
    println!("Compact serialized {} bytes", bytes_compact.len());
    let decoded_compact: Operation =
        rmp_serde::from_slice(&bytes_compact).expect("Compact deserialize failed");
    println!("Compact decoded successfully");

    // Test NAMED format (to_vec_named) for comparison
    let bytes_named = rmp_serde::to_vec_named(&op).expect("Named serialize failed");
    println!("Named serialized {} bytes", bytes_named.len());
    let decoded_named: Operation =
        rmp_serde::from_slice(&bytes_named).expect("Named deserialize failed");
    println!("Named decoded successfully");

    let decoded = decoded_compact;

    // Verify
    if let OpType::UpdateNodeType { node_type, .. } = &decoded.op_type {
        assert_eq!(node_type.name, "Article");
        assert_eq!(node_type.description, Some("Article node type".to_string()));
        println!("✅ Test passed!");
    } else {
        panic!("Wrong OpType variant");
    }
}

#[test]
fn test_archetype_in_operation_msgpack() {
    let archetype = Archetype {
        id: nanoid::nanoid!(16),
        name: "BlogPost".to_string(),
        extends: None,
        icon: None,
        title: None,
        description: Some("Blog post archetype".to_string()),
        base_node_type: None,
        fields: None,
        initial_content: None,
        layout: None,
        meta: None,
        version: None,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        publishable: None,
        strict: None,
        previous_version: None,
    };

    let mut vc = VectorClock::new();
    vc.increment("node1");

    let op = Operation::new(
        1,
        "node1".to_string(),
        vc,
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        OpType::UpdateArchetype {
            archetype_id: "BlogPost".to_string(),
            archetype,
        },
        "test_actor".to_string(),
    );

    let bytes = rmp_serde::to_vec_named(&op).expect("Archetype serialize failed");
    let _decoded: Operation = rmp_serde::from_slice(&bytes).expect("Archetype deserialize failed");
    println!("✅ Archetype test passed!");
}

#[test]
fn test_element_type_in_operation_msgpack() {
    let element_type = ElementType {
        id: nanoid::nanoid!(16),
        name: "Paragraph".to_string(),
        extends: None,
        title: None,
        icon: None,
        description: Some("Paragraph element type".to_string()),
        fields: Vec::new(),
        initial_content: None,
        layout: None,
        meta: None,
        version: None,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        publishable: None,
        strict: None,
        previous_version: None,
    };

    let mut vc = VectorClock::new();
    vc.increment("node1");

    let op = Operation::new(
        1,
        "node1".to_string(),
        vc,
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        OpType::UpdateElementType {
            element_type_id: "Paragraph".to_string(),
            element_type,
        },
        "test_actor".to_string(),
    );

    let bytes = rmp_serde::to_vec_named(&op).expect("ElementType serialize failed");
    let _decoded: Operation =
        rmp_serde::from_slice(&bytes).expect("ElementType deserialize failed");
    println!("✅ ElementType test passed!");
}
