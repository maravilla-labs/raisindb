// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! The ONE implementation of `raisin:Package` **node registration**.
//!
//! Package *content* writing was already unified — every caller enqueues a
//! `JobType::PackageInstall` handled by `raisin_rocksdb::PackageInstallHandler`.
//! Creating the `package-{name}` node that install job points at was not: five
//! call sites had grown five copies of "probe, build properties, write the
//! node", and they disagreed on the node id, the install mode, the binary's
//! tenant scope and whether an unchanged package is re-applied at all.
//!
//! This module owns those decisions. Callers supply a [`PackageNodeStore`]
//! adapter (their transaction / `NodeService` / repository handle — the write
//! mechanism legitimately differs) and everything above it is shared.
//!
//! ## The decisions, and why
//!
//! **Node id — always resolved from the path, never assumed.** The package's
//! identity is its path (`/{name}` in the `packages` workspace); the id is only
//! a handle. Packages that arrived through the streaming upload endpoint carry
//! a nanoid id, so any code that assumes `package-{name}` either creates a
//! duplicate or fails on the path conflict. [`resolve_package_node_id`] reuses
//! whatever id is already at the path and falls back to `package-{name}` for a
//! genuinely new registration.
//!
//! **Install mode — `skip` for a first install, `sync` for an update.**
//! `InstallMode::Sync` updates existing content while honouring the package's
//! own `manifest.yaml` `sync:` policy, so paths the package author marked
//! `skip` stay user-owned. `InstallMode::Overwrite` is the operator's *force*
//! escape hatch: it bypasses that policy entirely and deletes/replaces. A
//! definition-driven registration (builtin boot scan, system-update apply) is
//! never a force — see [`definition_install_mode`].
//!
//! **Binary tenant scope — always `Some(tenant_id)`.** The tenant prefix is
//! part of the generated blob key, so an unscoped `.rap` lands outside the
//! tenant's namespace and escapes per-tenant accounting and cleanup. Route the
//! store through [`store_package_blob`] and it cannot be forgotten.

use std::collections::HashMap;

use async_trait::async_trait;
use raisin_binary::{BinaryStorage, StoredObject};
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;
use raisin_packages::Manifest;

/// Workspace every `raisin:Package` node lives in.
pub const PACKAGES_WORKSPACE: &str = "packages";

/// NodeType of a package registration node.
pub const PACKAGE_NODE_TYPE: &str = "raisin:Package";

/// Path a package registers at, within [`PACKAGES_WORKSPACE`].
pub fn package_node_path(package_name: &str) -> String {
    format!("/{}", package_name)
}

/// The id a *new* package node gets. Existing nodes keep theirs — see
/// [`resolve_package_node_id`].
pub fn default_package_node_id(package_name: &str) -> String {
    format!("package-{}", package_name)
}

/// Which id a registration must write to.
///
/// Reuses the id already occupying the package's path. Assuming
/// `package-{name}` instead is what made re-registration fail for packages
/// uploaded through the streaming endpoint (nanoid ids): the write became a
/// CREATE that then collided on the unique path, stranding the tenant on the
/// stale bundle.
pub fn resolve_package_node_id(existing: Option<&Node>, package_name: &str) -> String {
    existing
        .map(|n| n.id.clone())
        .unwrap_or_else(|| default_package_node_id(package_name))
}

/// Where a registration came from. Decides the `builtin` / `content_hash`
/// bookkeeping properties.
#[derive(Debug, Clone)]
pub enum PackageOrigin {
    /// Shipped in the binary or the filesystem definition overlay — the boot
    /// scan and the system-update apply endpoint. The content hash is recorded
    /// on the node so the next pass can tell whether anything changed.
    Builtin { content_hash: String },
    /// An operator-supplied `.rap` upload.
    Uploaded,
    /// Built inside the repo from selected content.
    Generated,
}

/// The stored `.rap` blob a package node points at.
#[derive(Debug, Clone)]
pub struct PackageResource {
    pub key: String,
    pub url: String,
    pub size: usize,
}

impl PackageResource {
    pub fn new(key: impl Into<String>, url: impl Into<String>, size: usize) -> Self {
        Self {
            key: key.into(),
            url: url.into(),
            size,
        }
    }

    /// Build from what [`store_package_blob`] (or any `put_bytes`) returned.
    pub fn from_stored(stored: &StoredObject, size: usize) -> Self {
        Self {
            key: stored.key.clone(),
            url: stored.url.clone(),
            size,
        }
    }
}

/// Everything needed to register (create or update) one package node.
pub struct PackageRegistration<'a> {
    pub manifest: &'a Manifest,
    pub resource: PackageResource,
    pub origin: PackageOrigin,
}

/// Outcome of [`register_package_node`].
#[derive(Debug, Clone)]
pub struct RegisteredPackage {
    /// The id the node was written under.
    pub node_id: String,
    /// Whether a package node already occupied the path.
    pub existed: bool,
    /// `installed` flag of the node that was there before, if any.
    pub previously_installed: bool,
    /// `content_hash` of the node that was there before, if any.
    pub previous_content_hash: Option<String>,
}

impl RegisteredPackage {
    /// Install mode this registration's install job should run in.
    pub fn install_mode(&self) -> &'static str {
        definition_install_mode(self.existed)
    }
}

/// The `install_mode` job-context metadata value for a definition-driven
/// registration.
///
/// - a package that was not present → `"skip"`: nothing of the package's can be
///   in the repo yet, and `skip` must never clobber unrelated pre-existing
///   content that happens to share a path.
/// - a package that was present → `"sync"`: update what the package owns,
///   honouring the package's own `sync:` policy.
///
/// Deliberately never `"overwrite"`. Overwrite bypasses the package's `sync:`
/// policy entirely and deletes/replaces — it is the operator's explicit force,
/// reachable through `POST /packages/{name}/install?mode=overwrite`, not
/// something a definition refresh should choose on the operator's behalf.
pub fn definition_install_mode(existed: bool) -> &'static str {
    if existed {
        "sync"
    } else {
        "skip"
    }
}

/// Read the stored `.rap` blob key off an existing package node.
pub fn package_resource_key(node: &Node) -> Option<String> {
    match node.properties.get("resource") {
        Some(PropertyValue::Object(obj)) => match obj.get("key") {
            Some(PropertyValue::String(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Read the recorded definition content hash off an existing package node.
pub fn package_content_hash(node: &Node) -> Option<&str> {
    match node.properties.get("content_hash") {
        Some(PropertyValue::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// Whether a previous install job ran to completion. `false` means the node
/// exists but its content never landed — a broken install that must be redone.
pub fn package_is_installed(node: &Node) -> bool {
    matches!(
        node.properties.get("installed"),
        Some(PropertyValue::Boolean(true))
    )
}

/// Store a package's `.rap` bytes, always inside the tenant's blob namespace.
///
/// The tenant prefix is baked into the returned key, so passing `None` puts the
/// blob outside the tenant's namespace where it escapes per-tenant accounting
/// and cleanup. Existing keys keep resolving either way (the key is recorded on
/// the node verbatim), so routing every writer through here is a forward fix,
/// not a migration.
pub async fn store_package_blob<B: BinaryStorage + ?Sized>(
    binary_storage: &B,
    tenant_id: &str,
    manifest: &Manifest,
    zip_data: &[u8],
) -> anyhow::Result<StoredObject> {
    let filename = format!("{}-{}.rap", manifest.name, manifest.version);
    binary_storage
        .put_bytes(
            zip_data,
            Some("application/zip"),
            Some("rap"),
            Some(&filename),
            Some(tenant_id),
        )
        .await
}

/// The ONE `raisin:Package` property builder.
pub fn build_package_node_properties(
    reg: &PackageRegistration<'_>,
) -> HashMap<String, PropertyValue> {
    let manifest = reg.manifest;
    let mut properties = HashMap::new();

    properties.insert(
        "name".to_string(),
        PropertyValue::String(manifest.name.clone()),
    );
    properties.insert(
        "version".to_string(),
        PropertyValue::String(manifest.version.clone()),
    );
    if let Some(title) = &manifest.title {
        properties.insert("title".to_string(), PropertyValue::String(title.clone()));
    }
    if let Some(description) = &manifest.description {
        properties.insert(
            "description".to_string(),
            PropertyValue::String(description.clone()),
        );
    }
    if let Some(author) = &manifest.author {
        properties.insert("author".to_string(), PropertyValue::String(author.clone()));
    }
    if let Some(license) = &manifest.license {
        properties.insert(
            "license".to_string(),
            PropertyValue::String(license.clone()),
        );
    }
    // icon/color carry manifest defaults, so they are never absent.
    properties.insert(
        "icon".to_string(),
        PropertyValue::String(manifest.icon.clone()),
    );
    properties.insert(
        "color".to_string(),
        PropertyValue::String(manifest.color.clone()),
    );
    if !manifest.keywords.is_empty() {
        properties.insert(
            "keywords".to_string(),
            PropertyValue::Array(
                manifest
                    .keywords
                    .iter()
                    .map(|k| PropertyValue::String(k.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(category) = &manifest.category {
        properties.insert(
            "category".to_string(),
            PropertyValue::String(category.clone()),
        );
    }

    if !manifest.dependencies.is_empty() {
        let mut deps_map = HashMap::new();
        for (i, dep) in manifest.dependencies.iter().enumerate() {
            let mut dep_obj = HashMap::new();
            dep_obj.insert("name".to_string(), PropertyValue::String(dep.name.clone()));
            dep_obj.insert(
                "version".to_string(),
                PropertyValue::String(dep.version.clone()),
            );
            deps_map.insert(i.to_string(), PropertyValue::Object(dep_obj));
        }
        properties.insert("dependencies".to_string(), PropertyValue::Object(deps_map));
    }

    let provides = &manifest.provides;
    if !provides.nodetypes.is_empty()
        || !provides.workspaces.is_empty()
        || !provides.content.is_empty()
        || !provides.mixins.is_empty()
        || !provides.mcp_servers.is_empty()
    {
        let mut provides_obj = HashMap::new();
        let mut put_list = |key: &str, values: &[String]| {
            if !values.is_empty() {
                provides_obj.insert(
                    key.to_string(),
                    PropertyValue::Array(
                        values
                            .iter()
                            .map(|v| PropertyValue::String(v.clone()))
                            .collect(),
                    ),
                );
            }
        };
        put_list("nodetypes", &provides.nodetypes);
        put_list("mixins", &provides.mixins);
        put_list("workspaces", &provides.workspaces);
        put_list("content", &provides.content);
        put_list("mcp_servers", &provides.mcp_servers);
        properties.insert("provides".to_string(), PropertyValue::Object(provides_obj));
    }

    if !manifest.workspace_patches.is_empty() {
        let mut patches_obj = HashMap::new();
        for (ws_name, patch) in &manifest.workspace_patches {
            let mut patch_map = HashMap::new();
            if !patch.allowed_node_types.add.is_empty() {
                let mut ant_map = HashMap::new();
                ant_map.insert(
                    "add".to_string(),
                    PropertyValue::Array(
                        patch
                            .allowed_node_types
                            .add
                            .iter()
                            .map(|nt| PropertyValue::String(nt.clone()))
                            .collect(),
                    ),
                );
                patch_map.insert(
                    "allowed_node_types".to_string(),
                    PropertyValue::Object(ant_map),
                );
            }
            if let Some(default_folder_type) = &patch.default_folder_type {
                patch_map.insert(
                    "default_folder_type".to_string(),
                    PropertyValue::String(default_folder_type.clone()),
                );
            }
            patches_obj.insert(ws_name.clone(), PropertyValue::Object(patch_map));
        }
        properties.insert(
            "workspace_patches".to_string(),
            PropertyValue::Object(patches_obj),
        );
    }

    // Bookkeeping the registration itself owns.
    let is_builtin =
        matches!(reg.origin, PackageOrigin::Builtin { .. }) || manifest.builtin.unwrap_or(false);
    if is_builtin {
        properties.insert("builtin".to_string(), PropertyValue::Boolean(true));
    }
    if let PackageOrigin::Builtin { content_hash } = &reg.origin {
        // What the next boot's "did this package change?" check compares
        // against. Absent it, a rebuilt package never re-registers.
        properties.insert(
            "content_hash".to_string(),
            PropertyValue::String(content_hash.clone()),
        );
    }

    // The install job flips this to true when the content actually lands.
    properties.insert("installed".to_string(), PropertyValue::Boolean(false));

    let mut resource_obj = HashMap::new();
    resource_obj.insert(
        "key".to_string(),
        PropertyValue::String(reg.resource.key.clone()),
    );
    resource_obj.insert(
        "url".to_string(),
        PropertyValue::String(reg.resource.url.clone()),
    );
    resource_obj.insert(
        "mime_type".to_string(),
        PropertyValue::String("application/zip".to_string()),
    );
    resource_obj.insert(
        "size".to_string(),
        PropertyValue::Integer(reg.resource.size as i64),
    );
    properties.insert("resource".to_string(), PropertyValue::Object(resource_obj));

    properties
}

/// Build the `raisin:Package` node a registration writes.
///
/// Timestamps and authorship are deliberately left unset: they are stamped by
/// the low-level write layer, which also preserves the original `created_at` on
/// update.
pub fn build_package_node(reg: &PackageRegistration<'_>, existing: Option<&Node>) -> Node {
    let name = reg.manifest.name.clone();
    Node {
        id: resolve_package_node_id(existing, &name),
        node_type: existing
            .map(|n| n.node_type.clone())
            .unwrap_or_else(|| PACKAGE_NODE_TYPE.to_string()),
        path: package_node_path(&name),
        name,
        workspace: Some(PACKAGES_WORKSPACE.to_string()),
        properties: build_package_node_properties(reg),
        ..Default::default()
    }
}

/// The caller-specific half of registration: how to read and write a node in
/// the `packages` workspace.
///
/// Only the write *mechanism* varies (transaction vs `NodeService` vs
/// repository handle); the probe order, id resolution, property shape and
/// install-mode decision are shared above.
#[async_trait]
pub trait PackageNodeStore: Send + Sync {
    /// Look up the node currently at `path` in the `packages` workspace.
    ///
    /// Must probe by PATH. Probing by the assumed `package-{name}` id makes a
    /// nanoid-id package look absent, which downgrades an update to a first
    /// install (`install_mode = "skip"`) and silently drops the new content.
    async fn get_package_node_by_path(&self, path: &str) -> anyhow::Result<Option<Node>>;

    /// Persist the node. `existing` is what the probe returned, for stores that
    /// need to pick between create and update.
    async fn write_package_node(&self, node: Node, existing: Option<&Node>) -> anyhow::Result<()>;

    /// Whether a stored `.rap` blob still resolves. Used for zombie detection;
    /// stores that cannot check assume it does.
    async fn package_binary_exists(&self, _key: &str) -> bool {
        true
    }
}

/// Register (create or update) a package node.
///
/// Probes by path, resolves the id from what it finds, builds the properties
/// and writes. Returns what was there before so the caller can decide the
/// install mode ([`RegisteredPackage::install_mode`]) and its own bookkeeping.
pub async fn register_package_node<S>(
    store: &S,
    reg: &PackageRegistration<'_>,
) -> anyhow::Result<RegisteredPackage>
where
    S: PackageNodeStore + ?Sized,
{
    let path = package_node_path(&reg.manifest.name);
    let existing = store.get_package_node_by_path(&path).await?;

    let node = build_package_node(reg, existing.as_ref());
    let node_id = node.id.clone();

    store.write_package_node(node, existing.as_ref()).await?;

    Ok(RegisteredPackage {
        node_id,
        existed: existing.is_some(),
        previously_installed: existing.as_ref().map(package_is_installed).unwrap_or(false),
        previous_content_hash: existing
            .as_ref()
            .and_then(|n| package_content_hash(n).map(|s| s.to_string())),
    })
}

/// Whether an already-registered package still reflects `content_hash` **and**
/// its `.rap` blob is still retrievable.
///
/// The hash half stops a rebuilt package from silently keeping its stale
/// bundle; the blob half catches a "zombie" node whose binary was deleted
/// underneath it. Both are required — either alone has shipped a bug.
pub async fn package_registration_is_current<S>(
    store: &S,
    existing: Option<&Node>,
    content_hash: &str,
) -> bool
where
    S: PackageNodeStore + ?Sized,
{
    let Some(node) = existing else {
        return false;
    };
    if package_content_hash(node) != Some(content_hash) {
        return false;
    }
    match package_resource_key(node) {
        Some(key) => store.package_binary_exists(&key).await,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            name: "demo".to_string(),
            version: "1.2.3".to_string(),
            ..Default::default()
        }
    }

    fn registration(m: &Manifest) -> PackageRegistration<'_> {
        PackageRegistration {
            manifest: m,
            resource: PackageResource::new("t/2026/01/01/a.rap", "http://x/a.rap", 42),
            origin: PackageOrigin::Builtin {
                content_hash: "abc123".to_string(),
            },
        }
    }

    #[test]
    fn new_registration_uses_the_deterministic_id() {
        let m = manifest();
        let node = build_package_node(&registration(&m), None);
        assert_eq!(node.id, "package-demo");
        assert_eq!(node.path, "/demo");
        assert_eq!(node.node_type, PACKAGE_NODE_TYPE);
    }

    #[test]
    fn re_registration_reuses_the_existing_id() {
        let m = manifest();
        let existing = Node {
            id: "V1StGXR8_Z5jdHi6B-myT".to_string(),
            node_type: PACKAGE_NODE_TYPE.to_string(),
            path: "/demo".to_string(),
            ..Default::default()
        };
        let node = build_package_node(&registration(&m), Some(&existing));
        assert_eq!(
            node.id, "V1StGXR8_Z5jdHi6B-myT",
            "a nanoid-id package must be updated in place, not recreated"
        );
    }

    #[test]
    fn builtin_origin_records_the_content_hash() {
        let m = manifest();
        let props = build_package_node_properties(&registration(&m));
        assert_eq!(
            props.get("content_hash"),
            Some(&PropertyValue::String("abc123".to_string()))
        );
        assert_eq!(props.get("builtin"), Some(&PropertyValue::Boolean(true)));
        assert_eq!(props.get("installed"), Some(&PropertyValue::Boolean(false)));
    }

    #[test]
    fn uploaded_origin_records_no_content_hash() {
        let m = manifest();
        let reg = PackageRegistration {
            manifest: &m,
            resource: PackageResource::new("k", "u", 1),
            origin: PackageOrigin::Uploaded,
        };
        let props = build_package_node_properties(&reg);
        assert!(props.get("content_hash").is_none());
        assert!(props.get("builtin").is_none());
    }

    #[test]
    fn install_mode_is_never_overwrite() {
        assert_eq!(definition_install_mode(false), "skip");
        assert_eq!(definition_install_mode(true), "sync");
    }
}
