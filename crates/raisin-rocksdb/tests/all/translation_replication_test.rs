//! Plan Phase 11 item 4: translations replicate.
//!
//! Before it the repository captured an op no binary ever applied (and
//! captured `Hidden` as a delete), and the transaction captured nothing, so a
//! replica served untranslated content. Every version now travels as
//! `UpsertTranslationOverlay`. Ops here go through the PRODUCTION
//! receive path (`RocksDbOperationLogStorage::put_operations_batch`: persist,
//! apply, advance the vector clock).

use crate::translation_substrate_test::{code, get, rev, store, title, B, R, T, WS};
use raisin_context::RepositoryConfig;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::translations::LocaleOverlay;
use raisin_replication::{OpType, Operation, OperationLogStorage, ReplicatedOverlay, VectorClock};
use raisin_rocksdb::replication::RocksDbOperationLogStorage;
use raisin_rocksdb::{OpLogRepository, RocksDBConfig, RocksDBStorage};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, RegistryRepository, RepoScope, RepositoryManagementRepository, Storage,
    TranslationRepository, WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

pub(crate) const NODE: &str = "page-r";

pub(crate) struct Node {
    pub storage: Arc<RocksDBStorage>,
    pub id: String,
    _dir: TempDir,
}

/// A replicating storage that captures synchronously into its oplog.
pub(crate) async fn node(id: &str) -> Node {
    let dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = dir.path().to_path_buf();
    config.replication_enabled = true;
    config.async_operation_queue = false;
    config.cluster_node_id = Some(id.to_string());
    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());
    storage
        .registry()
        .register_tenant(T, HashMap::new())
        .await
        .unwrap();
    storage
        .repository_management()
        .create_repository(
            T,
            R,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".into(), "fr".into(), "de".into()],
                locale_fallback_chains: HashMap::new(),
                default_branch: B.to_string(),
                description: None,
                tags: HashMap::new(),
                localized_names: Default::default(),
            },
        )
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(T, R, B, "system", None, None, false, false)
        .await
        .unwrap();
    storage
        .workspaces()
        .put(
            RepoScope::new(T, R),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .unwrap();
    Node {
        storage,
        id: id.to_string(),
        _dir: dir,
    }
}

/// The translation ops `origin` captured after `after_seq`, in order.
pub(crate) fn translation_ops(origin: &Node, after_seq: u64) -> Vec<Operation> {
    let mut ops: Vec<Operation> = OpLogRepository::new(origin.storage.db().clone())
        .get_operations_from_node(T, R, &origin.id)
        .unwrap()
        .into_iter()
        .filter(|op| op.op_seq > after_seq)
        .filter(|op| matches!(op.op_type, OpType::UpsertTranslationOverlay { .. }))
        .collect();
    ops.sort_by_key(|op| op.op_seq);
    ops
}

pub(crate) fn highest_seq(origin: &Node) -> u64 {
    OpLogRepository::new(origin.storage.db().clone())
        .get_highest_seq(T, R, &origin.id)
        .unwrap()
}

pub(crate) async fn receive(replica: &Node, ops: &[Operation]) {
    RocksDbOperationLogStorage::new(replica.storage.clone())
        .put_operations_batch(ops)
        .await
        .unwrap();
}

pub(crate) fn peer_op(seq: u64, op_type: OpType) -> Operation {
    let mut op = Operation::new(
        seq,
        "newer-peer".to_string(),
        VectorClock::new(),
        T.to_string(),
        R.to_string(),
        B.to_string(),
        op_type,
        "peer-user".to_string(),
    );
    op.revision = Some(rev(seq));
    op
}

pub(crate) fn v2(locale: &str, overlay: ReplicatedOverlay, revision: HLC) -> OpType {
    OpType::UpsertTranslationOverlay {
        workspace: WS.to_string(),
        node_id: NODE.to_string(),
        locale: locale.to_string(),
        block_uuid: None,
        overlay,
        revision,
        history_complete_from: None,
    }
}

#[tokio::test]
async fn translation_replicates_to_peer() {
    let origin = node("origin").await;
    let replica = node("replica").await;

    // A repository write, and a transaction writing a node overlay and a
    // block overlay.
    store(&origin.storage, NODE, "fr", title("Bonjour"), rev(1)).await;
    let tx = origin.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(T, R).unwrap();
    tx.set_branch(B).unwrap();
    tx.set_message("translate").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.store_translation(WS, NODE, "de", title("Hallo"))
        .await
        .unwrap();
    tx.store_block_translation(WS, NODE, "block-1", "fr", title("Bloc"))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let ops = translation_ops(&origin, 0);
    assert_eq!(ops.len(), 3, "{ops:#?}");
    for op in &ops {
        let OpType::UpsertTranslationOverlay {
            workspace,
            revision,
            ..
        } = &op.op_type
        else {
            panic!("expected a translation version op: {:?}", op.op_type);
        };
        assert_eq!(workspace, WS);
        assert_eq!(op.revision, Some(*revision));
    }
    receive(&replica, &ops).await;

    let head = origin.storage.branches().get_head(T, R, B).await.unwrap();
    for locale in ["fr", "de"] {
        let on_origin = get(&origin.storage, NODE, locale, head).await.unwrap();
        assert!(on_origin.is_some());
        assert_eq!(
            get(&replica.storage, NODE, locale, head).await.unwrap(),
            on_origin
        );
    }
    assert_eq!(
        get(&replica.storage, NODE, "fr", rev(0)).await.unwrap(),
        None
    );
    let block = |s: &Arc<RocksDBStorage>| {
        let repo = s.translations().clone();
        async move {
            repo.get_block_translation(T, R, B, WS, NODE, "block-1", &code("fr"), &head)
                .await
                .unwrap()
        }
    };
    assert_eq!(block(&replica.storage).await, Some(title("Bloc")));
    assert_eq!(block(&replica.storage).await, block(&origin.storage).await);
    let meta = replica
        .storage
        .translations()
        .get_translation_meta(T, R, B, WS, NODE, &code("fr"))
        .await
        .unwrap()
        .expect("the replica records the version's meta");
    assert_eq!(meta.revision, rev(1));

    // A peer's op applies the same way.
    receive(
        &replica,
        &[peer_op(
            40,
            v2(
                "it",
                ReplicatedOverlay::from_stored(Some(&title("Ciao"))),
                rev(40),
            ),
        )],
    )
    .await;
    assert_eq!(
        get(&replica.storage, NODE, "it", rev(40)).await.unwrap(),
        Some(title("Ciao"))
    );
}

#[tokio::test]
async fn hidden_replicates_as_hidden() {
    let origin = node("origin").await;
    let replica = node("replica").await;
    store(&origin.storage, NODE, "fr", title("visible"), rev(1)).await;
    store(&origin.storage, NODE, "fr", LocaleOverlay::Hidden, rev(2)).await;

    let ops = translation_ops(&origin, 0);
    assert!(matches!(
        ops[1].op_type,
        OpType::UpsertTranslationOverlay {
            overlay: ReplicatedOverlay::Hidden,
            ..
        }
    ));
    receive(&replica, &ops).await;
    assert_eq!(
        get(&replica.storage, NODE, "fr", rev(2)).await.unwrap(),
        Some(LocaleOverlay::Hidden),
        "Hidden must arrive as Hidden, not as a delete"
    );
    assert_eq!(
        get(&replica.storage, NODE, "fr", rev(1)).await.unwrap(),
        Some(title("visible"))
    );

    // Hidden and Deleted are different states.
    receive(
        &replica,
        &[
            peer_op(10, v2("de", ReplicatedOverlay::Hidden, rev(10))),
            peer_op(
                11,
                v2(
                    "it",
                    ReplicatedOverlay::from_stored(Some(&title("x"))),
                    rev(11),
                ),
            ),
            peer_op(12, v2("it", ReplicatedOverlay::Deleted, rev(12))),
        ],
    )
    .await;
    assert_eq!(
        get(&replica.storage, NODE, "de", rev(12)).await.unwrap(),
        Some(LocaleOverlay::Hidden)
    );
    assert_eq!(
        get(&replica.storage, NODE, "it", rev(12)).await.unwrap(),
        None
    );
    assert_eq!(
        get(&replica.storage, NODE, "it", rev(11)).await.unwrap(),
        Some(title("x"))
    );
    let mut listed: Vec<String> = replica
        .storage
        .translations()
        .list_translations_for_node(T, R, B, WS, NODE, &rev(12))
        .await
        .unwrap()
        .iter()
        .map(|l| l.as_str().to_string())
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        vec!["de", "fr"],
        "a hidden locale is listed, a deleted one is not"
    );

    // A deletion (what the resync emits for `T`) deletes.
    receive(
        &replica,
        &[peer_op(13, v2("fr", ReplicatedOverlay::Deleted, rev(13)))],
    )
    .await;
    assert_eq!(
        get(&replica.storage, NODE, "fr", rev(13)).await.unwrap(),
        None
    );
}
