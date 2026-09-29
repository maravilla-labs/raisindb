// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Package install of pages whose blocks nest blocks, and of pages whose
//! references dangle.
//!
//! Regression 1: an accordion block (an element with a `uuid` and a field
//! named `items`) was stored as a `Composite`, which dropped its
//! `element_type` and every other field, and the page was rejected with
//! "Field '…content[0]' expects element values". Its translation overlay was
//! then skipped because the page did not exist, which is why this surfaced as
//! "nested block translations are rejected".
//!
//! Regression 2: one missing asset rejected the page using it and then every
//! page referencing that page, each error naming only its immediate target.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::Archetype;
use raisin_models::nodes::Node;
use raisin_models::translations::JsonPointer;
use raisin_storage::jobs::{JobId, JobRegistry};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    ArchetypeRepository, CommitMetadata, ElementTypeRepository, RepositoryManagementRepository,
    Storage,
};
use tempfile::TempDir;

use super::content_types::{ContentEntry, ContentNodeDef, InstallStats};
use super::handler::{rejection_report, PackageInstallHandler};
use super::translation::yaml_to_overlay;
use super::types::InstallMode;
use crate::RocksDBStorage;

const TENANT: &str = "default";
const REPO: &str = "testrepo";
const BRANCH: &str = "main";
const WS: &str = "default";

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
}

async fn setup() -> Env {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    storage
        .repository_management()
        .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    use raisin_storage::BranchRepository;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await
        .unwrap();
    raisin_core::nodetype_init::init_repository_nodetypes(storage.clone(), TENANT, REPO, BRANCH)
        .await
        .unwrap();
    raisin_core::workspace_init::init_repository_workspaces(storage.clone(), TENANT, REPO)
        .await
        .unwrap();

    let scope = || BranchScope::new(TENANT, REPO, BRANCH);
    for element_type in [
        serde_json::json!({
            "name": "test:RichText",
            "fields": [{ "$type": "TextField", "name": "text", "translatable": true }],
        }),
        serde_json::json!({
            "name": "test:AccordionItem",
            "fields": [
                { "$type": "TextField", "name": "title", "translatable": true },
                { "$type": "SectionField", "name": "content",
                  "allowed_element_types": ["test:RichText"] },
            ],
        }),
        // `items` + `uuid` on an element is the shape that was misread as a
        // Composite.
        serde_json::json!({
            "name": "test:Accordion",
            "fields": [
                { "$type": "TextField", "name": "headline", "translatable": true },
                { "$type": "SectionField", "name": "items",
                  "allowed_element_types": ["test:AccordionItem"] },
            ],
        }),
    ] {
        let element_type: ElementType = serde_json::from_value(element_type).unwrap();
        storage
            .element_types()
            .create(scope(), element_type, CommitMetadata::system("seed"))
            .await
            .unwrap();
    }

    let archetype: Archetype = serde_json::from_value(serde_json::json!({
        "name": "test:ContentPage",
        "base_node_type": "raisin:Page",
        "fields": [
            { "$type": "SectionField", "name": "blocks",
              "allowed_element_types": ["test:Accordion", "test:RichText"] },
        ],
    }))
    .unwrap();
    storage
        .archetypes()
        .create(scope(), archetype, CommitMetadata::system("seed"))
        .await
        .unwrap();

    Env { _dir: dir, storage }
}

/// A `.node.yaml` entry, parsed exactly as the zip collector parses it.
fn node_entry(node_path: &str, yaml: &str) -> ContentEntry {
    let def: ContentNodeDef = serde_yaml::from_str(yaml).unwrap();
    ContentEntry::NodeDef {
        workspace: WS.to_string(),
        yaml_path: format!("content/{WS}{node_path}/.node.yaml"),
        node: Box::new(Node {
            id: nanoid::nanoid!(),
            node_type: def.node_type,
            name: node_path.rsplit('/').next().unwrap().to_string(),
            path: node_path.to_string(),
            workspace: Some(WS.to_string()),
            archetype: def.archetype,
            properties: def.properties.unwrap_or_default(),
            ..Default::default()
        }),
        legacy_path: None,
    }
}

fn translation_entry(node_path: &str, locale: &str, yaml: &str) -> ContentEntry {
    let value: serde_json::Value = serde_yaml::from_str(yaml).unwrap();
    ContentEntry::TranslationFile {
        workspace: WS.to_string(),
        base_node_yaml_path: format!("content/{WS}{node_path}/.node.yaml"),
        locale: locale.to_string(),
        overlay: yaml_to_overlay(value).unwrap(),
    }
}

async fn install(env: &Env, entries: Vec<ContentEntry>) -> InstallStats {
    let mut stats = InstallStats::default();
    PackageInstallHandler::new(env.storage.clone(), Arc::new(JobRegistry::new()))
        .install_sorted_entries(
            entries,
            &HashMap::new(),
            TENANT,
            REPO,
            BRANCH,
            &JobId::new(),
            InstallMode::Sync,
            None,
            &HashMap::new(),
            None,
            &mut stats,
        )
        .await
        .expect("per-entry rejections are collected, not returned");
    stats
}

async fn read(env: &Env, path: &str) -> Option<Node> {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(BRANCH).unwrap();
    tx.get_node_by_path(WS, path).await.unwrap()
}

const SITE_FOLDER: &str = "node_type: raisin:Folder\n";

// The accordion is written in the legacy wrapped form
// (`{element_type, uuid, content: {...}}`) the migration materializer emits;
// its item has a real field named `content` holding nested blocks.
const FAQ_PAGE: &str = r#"
node_type: raisin:Page
archetype: test:ContentPage
properties:
  title: Häufige Fragen
  blocks:
    - element_type: test:Accordion
      uuid: acc-1
      content:
        headline: Finden Sie Antworten
        items:
          - element_type: test:AccordionItem
            uuid: acc-1-i0
            title: Muss man Parkplätze reservieren?
            content:
              - element_type: test:RichText
                uuid: acc-1-i0-c0
                text: <p>Ja.</p>
"#;

const FAQ_FR: &str = r#"
title: Questions fréquentes
blocks:
  - uuid: acc-1
    headline: Trouvez les réponses
    items:
      - uuid: acc-1-i0
        title: Faut-il réserver une place?
        content:
          - uuid: acc-1-i0-c0
            text: <p>Oui.</p>
"#;

#[tokio::test]
async fn page_with_nested_blocks_and_nested_overlay_installs() {
    let env = setup().await;

    let stats = install(
        &env,
        vec![
            node_entry("/site", SITE_FOLDER),
            node_entry("/site/faq", FAQ_PAGE),
            translation_entry("/site/faq", "fr", FAQ_FR),
        ],
    )
    .await;

    assert!(
        stats.content_errors.is_empty() && stats.content_errors_cascaded.is_empty(),
        "rejected: {:?} {:?}",
        stats.content_errors,
        stats.content_errors_cascaded
    );
    assert_eq!(stats.translations_applied, 1, "the fr overlay must land");

    let page = read(&env, "/site/faq").await.expect("page installed");
    let Some(PropertyValue::Array(blocks)) = page.properties.get("blocks") else {
        panic!("blocks missing: {:?}", page.properties.get("blocks"));
    };
    let PropertyValue::Element(accordion) = &blocks[0] else {
        panic!("blocks[0] must stay an element, got {:?}", blocks[0]);
    };
    assert_eq!(accordion.element_type, "test:Accordion");
    assert_eq!(accordion.uuid, "acc-1");
    assert_eq!(
        accordion.content.get("headline"),
        Some(&PropertyValue::String("Finden Sie Antworten".into()))
    );
    let Some(PropertyValue::Array(items)) = accordion.content.get("items") else {
        panic!("accordion items missing: {:?}", accordion.content);
    };
    let PropertyValue::Element(item) = &items[0] else {
        panic!("items[0] must be an element, got {:?}", items[0]);
    };
    assert_eq!(item.element_type, "test:AccordionItem");
    assert!(matches!(item.content.get("content"), Some(PropertyValue::Array(c)) if c.len() == 1));

    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(BRANCH).unwrap();
    let overlay = tx
        .get_translation(WS, &page.id, "fr")
        .await
        .unwrap()
        .expect("fr overlay stored");
    let data = overlay.properties_ref().unwrap();
    assert_eq!(
        data.get(&JsonPointer::new(
            "/blocks/acc-1/items/acc-1-i0/content/acc-1-i0-c0/text"
        )),
        Some(&PropertyValue::String("<p>Oui.</p>".into())),
        "the translation of a block nested two levels deep is addressed by uuid"
    );
}

#[tokio::test]
async fn cascaded_rejections_name_the_root_cause() {
    let env = setup().await;

    let stats = install(
        &env,
        vec![
            node_entry("/site", SITE_FOLDER),
            node_entry(
                "/site/a",
                "node_type: raisin:Page\nproperties:\n  title: A\n  image:\n    raisin:ref: /media/missing.jpg\n    raisin:workspace: default\n",
            ),
            node_entry(
                "/site/b",
                "node_type: raisin:Page\nproperties:\n  title: B\n  related:\n    raisin:ref: /site/a\n    raisin:workspace: default\n",
            ),
            node_entry(
                "/site/c",
                "node_type: raisin:Page\nproperties:\n  title: C\n  related:\n    raisin:ref: /site/b\n    raisin:workspace: default\n",
            ),
        ],
    )
    .await;

    assert_eq!(stats.content_errors.len(), 1, "{:?}", stats.content_errors);
    let root = &stats.content_errors[0];
    assert!(
        root.contains("default:/site/a")
            && root.contains("Referenced node not found: default:/media/missing.jpg")
            && root.contains("(at image)"),
        "root cause names the page, the missing target and the property: {root}"
    );

    assert_eq!(
        stats.content_errors_cascaded.len(),
        2,
        "{:?}",
        stats.content_errors_cascaded
    );
    for (entry, target) in [("/site/b", "/site/a"), ("/site/c", "/site/b")] {
        let line = stats
            .content_errors_cascaded
            .iter()
            .find(|l| l.starts_with(&format!("default:{entry} ")))
            .unwrap_or_else(|| panic!("no cascade line for {entry}"));
        assert!(
            line.contains(&format!("references default:{target}, which was rejected"))
                && line.contains("default:/media/missing.jpg"),
            "cascade names its target and the root cause: {line}"
        );
    }

    let report = rejection_report("site", 1, &stats);
    let root_at = report.find("missing.jpg (at image)").unwrap();
    let cascade_at = report
        .find("Rejected because they reference a rejected entry:")
        .expect("cascades are listed under their own heading");
    assert!(root_at < cascade_at, "root causes come first:\n{report}");
    assert!(
        report.contains("3 were rejected (1 root cause(s); 2 more"),
        "{report}"
    );
}
