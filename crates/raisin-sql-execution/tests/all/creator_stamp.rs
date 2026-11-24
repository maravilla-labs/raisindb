//! The creator: `created_by` is stamped from the caller on create (a client
//! value is ignored), kept by a later (partial) update of someone else, and
//! readable as a column — what a function reads through `nodes.get`.

use super::write_grants_rls::{engine, exec, setup, user, BRANCH, REPO, TENANT, WS};
use futures::StreamExt;
use raisin_models::permissions::{Operation, Permission};
use raisin_storage::{NodeRepository, Storage, StorageScope};

#[tokio::test]
async fn created_by_names_the_creating_user_and_survives_updates() {
    let (storage, _dir) = setup().await;
    let all = || {
        Permission::new(
            "/**",
            vec![Operation::Create, Operation::Read, Operation::Update],
        )
    };
    let alice = engine(&storage, user("user-alice", vec![all()], None));
    let bob = engine(&storage, user("user-bob", vec![all()], None));

    exec(
        &alice,
        "INSERT INTO items (path, node_type, created_by, properties) VALUES \
         ('/wo-9', 'test:WorkOrder', 'forged', '{\"status\":\"open\"}'::JSONB)",
    )
    .await
    .expect("create");
    exec(
        &bob,
        "UPDATE items SET properties = properties || '{\"status\":\"done\"}'::JSONB WHERE path = '/wo-9'",
    )
    .await
    .expect("update");

    let node = storage
        .nodes()
        .get_by_path(StorageScope::new(TENANT, REPO, BRANCH, WS), "/wo-9", None)
        .await
        .expect("read")
        .expect("node");
    assert_eq!(node.created_by.as_deref(), Some("user-alice"));
    assert_eq!(node.updated_by.as_deref(), Some("user-bob"));

    let mut rows = alice
        .execute("SELECT created_by FROM items WHERE path = '/wo-9'")
        .await
        .expect("select");
    let row = rows.next().await.expect("a row").expect("row ok");
    let value = format!("{:?}", row);
    assert!(value.contains("user-alice"), "{value}");
}
