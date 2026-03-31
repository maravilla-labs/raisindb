//! Tests for the connection API.

use super::*;
use raisin_storage_memory::InMemoryStorage;
use std::sync::Arc;

#[test]
fn test_connection_creation() {
    let storage = Arc::new(InMemoryStorage::default());
    let connection = RaisinConnection::with_storage(storage);

    let tenant = connection.tenant("acme-corp");
    assert_eq!(tenant.tenant_id(), "acme-corp");
}

#[test]
fn test_repository_access() {
    let storage = Arc::new(InMemoryStorage::default());
    let connection = RaisinConnection::with_storage(storage);

    let tenant = connection.tenant("acme-corp");
    let repo = tenant.repository("website");

    assert_eq!(repo.tenant_id(), "acme-corp");
    assert_eq!(repo.repo_id(), "website");
    assert_eq!(repo.context().storage_prefix(), "/acme-corp/repo/website");
}

#[test]
fn test_workspace_access() {
    let storage = Arc::new(InMemoryStorage::default());
    let connection = RaisinConnection::with_storage(storage);

    let tenant = connection.tenant("acme-corp");
    let repo = tenant.repository("website");
    let workspace = repo.workspace("main");

    assert_eq!(workspace.workspace_id(), "main");
}

#[test]
fn test_node_service_builder() {
    let storage = Arc::new(InMemoryStorage::default());
    let connection = RaisinConnection::with_storage(storage);

    let tenant = connection.tenant("acme-corp");
    let repo = tenant.repository("website");
    let workspace = repo.workspace("main");

    let nodes = workspace.nodes();
    assert_eq!(nodes.effective_branch(), "main");

    let develop = workspace.nodes().branch("develop");
    assert_eq!(develop.effective_branch(), "develop");
}
