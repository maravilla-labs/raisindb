//! The startup resync decides "is this branch up to date?" by comparing the
//! STORED NodeType with the built-in definition. That only works if a
//! definition written to RocksDB reads back equal (modulo what storage owns:
//! id, timestamps, the bumped version). If any built-in did not, the resync
//! would rewrite it — and mint a revision — on every boot, on every branch.
//! This pins the round trip for every built-in, and the outage scenario on
//! the real store: an old raisin:AclFolder on `main` while another branch has
//! already recorded the applied hash.

use raisin_core::definitions::DefinitionResolver;
use raisin_core::system_updates::{
    check_pending_updates, resync_nodetypes, stored_nodetype_matches, AutoApplyPolicy,
};
use raisin_rocksdb::{RocksDBStorage, SystemUpdateRepositoryImpl};
use raisin_storage::scope::BranchScope;
use raisin_storage::system_updates::ResourceType;
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeTypeRepository, RepositoryManagementRepository, Storage,
};
use std::sync::Arc;

const TENANT: &str = "t";
const REPO: &str = "r";

#[tokio::test]
#[allow(deprecated)] // `put` is what the system-update apply path itself calls
async fn builtin_nodetypes_round_trip_and_a_stale_main_is_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    storage
        .repository_management()
        .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    for b in ["draft", "main"] {
        storage
            .branches()
            .create_branch(TENANT, REPO, b, "test", None, None, false, false)
            .await
            .unwrap();
    }
    let updates = SystemUpdateRepositoryImpl::new(storage.db().clone());
    let resolver = DefinitionResolver::embedded_only();
    let defs = resolver.nodetypes();

    for branch in ["draft", "main"] {
        resync_nodetypes(
            storage.clone(),
            &updates,
            TENANT,
            REPO,
            branch,
            &defs,
            AutoApplyPolicy::NonBreaking,
        )
        .await
        .unwrap();
    }

    // Every built-in reads back as "in sync", so a second boot writes nothing.
    for (def, _) in &defs {
        let stored = storage
            .node_types()
            .get(BranchScope::new(TENANT, REPO, "main"), &def.name, None)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{} missing on main", def.name));
        assert!(
            stored_nodetype_matches(def, &stored),
            "{} does not round-trip through RocksDB; the resync would rewrite it on every boot",
            def.name
        );
    }
    let again = resync_nodetypes(
        storage.clone(),
        &updates,
        TENANT,
        REPO,
        "main",
        &defs,
        AutoApplyPolicy::NonBreaking,
    )
    .await
    .unwrap();
    assert_eq!(again.applied, 0);

    // The outage: main holds an AclFolder from before the messaging records.
    let (acl, _) = defs
        .iter()
        .find(|(nt, _)| nt.name == "raisin:AclFolder")
        .unwrap()
        .clone();
    let mut old = acl.clone();
    old.allowed_children
        .retain(|c| c != "raisin:ConversationRecord");
    old.allowed_children.push("acme:Custom".to_string());
    storage
        .node_types()
        .put(
            BranchScope::new(TENANT, REPO, "main"),
            old,
            CommitMetadata::system("old"),
        )
        .await
        .unwrap();

    let pending = check_pending_updates(storage.clone(), &updates, &resolver, TENANT, REPO, "main")
        .await
        .unwrap();
    let stale: Vec<_> = pending
        .updates
        .iter()
        .filter(|u| u.resource_type == ResourceType::NodeType)
        .map(|u| u.name.clone())
        .collect();
    assert_eq!(stale, vec!["raisin:AclFolder".to_string()]);

    let outcome = resync_nodetypes(
        storage.clone(),
        &updates,
        TENANT,
        REPO,
        "main",
        &defs,
        AutoApplyPolicy::NonBreaking,
    )
    .await
    .unwrap();
    assert_eq!(outcome.applied, 1);
    let repaired = storage
        .node_types()
        .get(
            BranchScope::new(TENANT, REPO, "main"),
            "raisin:AclFolder",
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(repaired
        .allowed_children
        .iter()
        .any(|c| c == "raisin:ConversationRecord"));
    assert!(repaired.allowed_children.iter().any(|c| c == "acme:Custom"));
}
