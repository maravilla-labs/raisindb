use raisin_context::Branch;
use raisin_hlc::HLC;
use raisin_models::admin_user::DatabaseAdminUser;
use raisin_models::auth::{Identity, Session};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::nodes::{element::element_type::ElementType, RelationRef};
use raisin_models::registry::{DeploymentRegistration, TenantRegistration};
use raisin_models::workspace::Workspace;
use raisin_storage::RevisionMeta;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use super::ReplicatedNodeChange;

// NOTE: This enum intentionally exceeds 300 lines - it is a single enum definition
// with many variants that cannot be further decomposed in Rust.

/// The type of operation being performed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OpType {
    /// Create a new node (storage node in the database)
    CreateNode {
        node_id: String, // Storage node ID
        name: String,
        node_type: String,
        archetype: Option<String>,
        parent_id: Option<String>,
        order_key: String,
        #[serde(default)]
        properties: HashMap<String, PropertyValue>,
        owner_id: Option<String>,
        workspace: Option<String>,
        #[serde(default)]
        path: String, // Full path to the node (e.g., "/content/page")
    },

    /// Delete an existing node (storage node)
    DeleteNode {
        node_id: String, // Storage node ID
    },

    /// Set a single property on a node (granular for CRDT)
    SetProperty {
        node_id: String, // Storage node ID
        property_name: String,
        value: PropertyValue,
    },

    /// Delete a single property from a node
    DeleteProperty {
        node_id: String, // Storage node ID
        property_name: String,
    },

    /// Rename a node
    RenameNode {
        node_id: String,
        old_name: String,
        new_name: String,
    },

    /// Change node archetype
    SetArchetype {
        node_id: String,
        old_archetype: Option<String>,
        new_archetype: Option<String>,
    },

    /// Update node order key (for sibling ordering)
    SetOrderKey {
        node_id: String,
        old_order_key: String,
        new_order_key: String,
    },

    /// Transfer node ownership
    SetOwner {
        node_id: String,
        old_owner_id: Option<String>,
        new_owner_id: Option<String>,
    },

    /// Publish a node
    PublishNode {
        node_id: String,
        published_by: String,
        published_at: u64, // timestamp_ms
    },

    /// Unpublish a node
    UnpublishNode { node_id: String },

    /// Set translation for a property
    SetTranslation {
        node_id: String,
        locale: String,
        property_name: String,
        value: PropertyValue,
    },

    /// Delete translation for a property
    DeleteTranslation {
        node_id: String,
        locale: String,
        property_name: String,
    },

    /// Add a relation between nodes (Last-Write-Wins CRDT)
    ///
    /// Relations are identified by the composite key (source_id, target_id, relation_type).
    /// Only one relation of a given type can exist between two nodes.
    /// Concurrent updates are resolved using HLC timestamps (LWW).
    AddRelation {
        source_id: String,
        source_workspace: String,
        relation_type: String,
        target_id: String,
        target_workspace: String,
        relation: RelationRef,
    },

    /// Remove a relation between nodes (Last-Write-Wins CRDT)
    ///
    /// Identified by the composite key (source_id, target_id, relation_type).
    RemoveRelation {
        source_id: String,
        source_workspace: String,
        relation_type: String,
        target_id: String,
        target_workspace: String,
    },

    /// Move a node to a new parent (Last-Write-Wins)
    MoveNode {
        node_id: String, // Storage node ID
        old_parent_id: Option<String>,
        new_parent_id: Option<String>,
        /// Fractional index for ordering among siblings
        position: Option<String>,
    },
    /// Apply a fully materialized revision captured at commit time
    ApplyRevision {
        /// Target branch head after applying the revision
        branch_head: HLC,
        /// Batched node-level mutations in commit order
        node_changes: Vec<ReplicatedNodeChange>,
    },

    /// Upsert a node snapshot (decomposed from ApplyRevision for CRDT commutativity)
    /// This operation is LWW-based and commutative with other node operations
    UpsertNodeSnapshot {
        node: Node,
        parent_id: Option<String>,
        revision: HLC,
        cf_order_key: String,
    },

    /// Delete a node snapshot (decomposed from ApplyRevision for CRDT commutativity)
    /// This operation is Delete-Wins and commutative with other node operations
    DeleteNodeSnapshot { node_id: String, revision: HLC },

    /// Insert an element into an ordered list (RGA CRDT)
    ListInsertAfter {
        node_id: String, // Storage node ID
        list_property: String,
        /// Element to insert after (None = insert at beginning)
        after_id: Option<Uuid>,
        value: PropertyValue,
        /// Unique immutable ID for this list element
        element_id: Uuid,
    },

    /// Delete an element from an ordered list (RGA CRDT)
    ListDelete {
        node_id: String, // Storage node ID
        list_property: String,
        /// The element_id from ListInsertAfter
        element_id: Uuid,
    },

    /// Update a NodeType schema
    UpdateNodeType {
        node_type_id: String,
        node_type: NodeType,
    },

    /// Delete a NodeType schema
    DeleteNodeType { node_type_id: String },

    /// Update an Archetype
    UpdateArchetype {
        archetype_id: String,
        archetype: Archetype,
    },

    /// Delete an Archetype
    DeleteArchetype { archetype_id: String },

    /// Update an ElementType
    UpdateElementType {
        element_type_id: String,
        element_type: ElementType,
    },

    /// Delete an ElementType
    DeleteElementType { element_type_id: String },

    /// Create or update a workspace
    UpdateWorkspace {
        workspace_id: String,
        workspace: Workspace,
    },

    /// Delete a workspace
    DeleteWorkspace { workspace_id: String },

    /// Create or update a branch
    UpdateBranch { branch: Branch },

    /// Create revision metadata (for revision history/log)
    /// This metadata is essential for displaying commit history and tracking changes
    CreateRevisionMeta { revision_meta: RevisionMeta },

    /// Delete a branch
    DeleteBranch { branch_id: String },

    /// Create a tag pointing to a revision (HLC format: "timestamp-counter")
    CreateTag { tag_name: String, revision: String },

    /// Delete a tag
    DeleteTag { tag_name: String },

    /// Create or update a user
    UpdateUser {
        user_id: String,
        user: DatabaseAdminUser,
    },

    /// Delete a user
    DeleteUser { user_id: String },

    /// Create or update a tenant
    UpdateTenant {
        tenant_id: String,
        tenant: TenantRegistration,
    },

    /// Delete a tenant
    DeleteTenant { tenant_id: String },

    /// Create or update a deployment
    UpdateDeployment {
        deployment_id: String,
        deployment: DeploymentRegistration,
    },

    /// Delete a deployment
    DeleteDeployment { deployment_id: String },

    /// Create or update a repository within a tenant
    UpdateRepository {
        tenant_id: String,
        repo_id: String,
        repository: raisin_context::RepositoryInfo,
    },

    /// Delete a repository
    DeleteRepository { tenant_id: String, repo_id: String },

    /// Grant permission
    GrantPermission {
        subject_type: String, // "user" | "role" | "group"
        subject_id: String,
        resource_type: String,
        resource_id: String,
        permission: String,
    },

    /// Revoke permission
    RevokePermission {
        subject_type: String,
        subject_id: String,
        resource_type: String,
        resource_id: String,
        permission: String,
    },

    // =========================================================================
    // Identity & Session Operations (for pluggable authentication)
    // =========================================================================
    /// Create or update an identity
    ///
    /// Identities are global per tenant and can have multiple authentication
    /// providers linked. This operation uses LWW semantics.
    UpsertIdentity {
        identity_id: String,
        identity: Identity,
    },

    /// Delete an identity
    ///
    /// Removes an identity and should also clean up associated sessions.
    DeleteIdentity { identity_id: String },

    /// Create a new session
    ///
    /// Sessions track active authentication and are linked to identities.
    CreateSession {
        session_id: String,
        session: Session,
    },

    /// Revoke a session
    ///
    /// Terminates an active session, invalidating the associated tokens.
    RevokeSession { session_id: String },

    /// Revoke all sessions for an identity
    ///
    /// Bulk session revocation, typically used when deactivating an identity
    /// or when password is changed.
    RevokeAllIdentitySessions { identity_id: String },

    /// Rotate refresh token (increment generation counter)
    ///
    /// This operation is captured when a token is refreshed, incrementing the
    /// generation counter to detect token reuse attacks across cluster nodes.
    RotateRefreshToken {
        session_id: String,
        new_generation: u32,
    },
}
