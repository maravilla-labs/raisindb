//! Access Control statement execution.
//!
//! Handles all ACL statement variants: CREATE/ALTER/DROP/SHOW/DESCRIBE for
//! ROLE, GROUP, USER, plus GRANT, REVOKE, ALTER SECURITY CONFIG,
//! SHOW SECURITY CONFIG, SHOW PERMISSIONS FOR, and SHOW EFFECTIVE ROLES FOR.
//!
//! ACL entities are stored as nodes in the `raisin:access_control` workspace:
//! - Roles at `/roles/{role_id}` with node_type `raisin:Role`
//! - Groups at `/groups/{group_id}` with node_type `raisin:Group`
//! - Users at `/users/{user_id}` with node_type `raisin:User`
//! - Security configs at `/config/{name}` with node_type `raisin:SecurityConfig`

use super::QueryEngine;
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql::ast::acl::*;
use raisin_storage::node_operations::{
    CreateNodeOptions, DeleteNodeOptions, ListOptions, UpdateNodeOptions,
};
use raisin_storage::scope::StorageScope;
use raisin_storage::traits::node::NodeRepository;
use raisin_storage::Storage;
use std::collections::HashMap;

/// The workspace where all ACL entities are stored.
const ACL_WORKSPACE: &str = "raisin:access_control";

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Check that the current user is authorized to execute an ACL statement.
    ///
    /// Mutating operations (CREATE, ALTER, DROP, GRANT, REVOKE) require the
    /// `system_admin` role or write access to the `raisin:access_control` workspace.
    /// Read operations (SHOW, DESCRIBE) require authentication (not anonymous).
    fn acl_check_authorization(&self, stmt: &AclStatement) -> Result<(), Error> {
        let auth = self.auth_context.as_ref().ok_or_else(|| {
            Error::Forbidden(
                "Authentication required for access control operations".to_string(),
            )
        })?;

        // System context always allowed (internal operations)
        if auth.is_system {
            return Ok(());
        }

        // Anonymous users cannot perform any ACL operations
        if auth.is_anonymous {
            return Err(Error::Forbidden(
                "Anonymous users cannot perform access control operations".to_string(),
            ));
        }

        // Read-only operations (SHOW, DESCRIBE) only require authentication
        if stmt.is_read_only() {
            return Ok(());
        }

        // Mutating operations require system_admin role or resolved write permissions
        // on the raisin:access_control workspace
        if let Some(ref perms) = auth.resolved_permissions {
            if perms.is_system_admin {
                return Ok(());
            }
        }

        // Check if user has the system_admin role directly
        if auth.has_role("system_admin") {
            return Ok(());
        }

        // No system_admin access -- deny mutating operations
        Err(Error::Forbidden(format!(
            "Insufficient permissions: {} requires the system_admin role",
            stmt.operation()
        )))
    }

    /// Execute an ACL statement.
    pub(crate) async fn execute_acl(&self, stmt: &AclStatement) -> Result<RowStream, Error> {
        tracing::info!("Executing ACL statement: {}", stmt.operation());

        // Authorization check -- enforce before any mutations
        self.acl_check_authorization(stmt)?;

        match stmt {
            AclStatement::CreateRole(s) => self.acl_create_role(s).await,
            AclStatement::AlterRole(s) => self.acl_alter_role(s).await,
            AclStatement::DropRole(s) => self.acl_drop_role(s).await,
            AclStatement::ShowRoles(s) => self.acl_show_roles(s).await,
            AclStatement::DescribeRole(s) => self.acl_describe_role(s).await,

            AclStatement::CreateGroup(s) => self.acl_create_group(s).await,
            AclStatement::AlterGroup(s) => self.acl_alter_group(s).await,
            AclStatement::DropGroup(s) => self.acl_drop_group(s).await,
            AclStatement::ShowGroups(s) => self.acl_show_groups(s).await,
            AclStatement::DescribeGroup(s) => self.acl_describe_group(s).await,

            AclStatement::CreateUser(s) => self.acl_create_user(s).await,
            AclStatement::AlterUser(s) => self.acl_alter_user(s).await,
            AclStatement::DropUser(s) => self.acl_drop_user(s).await,
            AclStatement::ShowUsers(s) => self.acl_show_users(s).await,
            AclStatement::DescribeUser(s) => self.acl_describe_user(s).await,

            AclStatement::Grant(s) => self.acl_grant(s).await,
            AclStatement::Revoke(s) => self.acl_revoke(s).await,

            AclStatement::AlterSecurityConfig(s) => self.acl_alter_security_config(s).await,
            AclStatement::ShowSecurityConfig(s) => self.acl_show_security_config(s).await,

            AclStatement::ShowPermissionsFor(s) => self.acl_show_permissions_for(s).await,
            AclStatement::ShowEffectiveRolesFor(s) => self.acl_show_effective_roles_for(s).await,
        }
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    fn acl_scope(&self) -> StorageScope<'_> {
        StorageScope {
            tenant_id: &self.tenant_id,
            repo_id: &self.repo_id,
            branch: &self.branch,
            workspace: ACL_WORKSPACE,
        }
    }

    fn acl_create_opts() -> CreateNodeOptions {
        CreateNodeOptions {
            validate_schema: false,
            validate_parent_allows_child: false,
            validate_workspace_allows_type: false,
            ..Default::default()
        }
    }

    fn acl_update_opts() -> UpdateNodeOptions {
        UpdateNodeOptions {
            validate_schema: false,
            ..Default::default()
        }
    }

    fn acl_ok(message: impl Into<String>) -> Result<RowStream, Error> {
        let mut row = Row::new();
        row.insert("result".to_string(), PropertyValue::String(message.into()));
        row.insert("success".to_string(), PropertyValue::Boolean(true));
        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    fn acl_rows(rows: Vec<Row>) -> Result<RowStream, Error> {
        let results: Vec<Result<Row, Error>> = rows.into_iter().map(Ok).collect();
        Ok(Box::pin(stream::iter(results)))
    }

    /// Convert a permission grant to a PropertyValue::Object.
    fn perm_to_pv(grant: &PermissionGrant) -> PropertyValue {
        let mut obj = HashMap::new();
        obj.insert(
            "path".to_string(),
            PropertyValue::String(grant.path.clone()),
        );
        obj.insert(
            "operations".to_string(),
            PropertyValue::Array(
                grant
                    .operations
                    .iter()
                    .map(|op| PropertyValue::String(op.to_string().to_lowercase()))
                    .collect(),
            ),
        );
        if let Some(ref ws) = grant.workspace {
            obj.insert("workspace".to_string(), PropertyValue::String(ws.clone()));
        }
        if let Some(ref bp) = grant.branch_pattern {
            obj.insert(
                "branch_pattern".to_string(),
                PropertyValue::String(bp.clone()),
            );
        }
        if let Some(ref nt) = grant.node_types {
            obj.insert(
                "node_types".to_string(),
                PropertyValue::Array(
                    nt.iter()
                        .map(|t| PropertyValue::String(t.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(ref f) = grant.fields {
            obj.insert(
                "fields".to_string(),
                PropertyValue::Array(f.iter().map(|s| PropertyValue::String(s.clone())).collect()),
            );
        }
        if let Some(ref ef) = grant.except_fields {
            obj.insert(
                "except_fields".to_string(),
                PropertyValue::Array(
                    ef.iter()
                        .map(|s| PropertyValue::String(s.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(ref c) = grant.condition {
            obj.insert("condition".to_string(), PropertyValue::String(c.clone()));
        }
        PropertyValue::Object(obj)
    }

    fn strings_to_pv(items: &[String]) -> PropertyValue {
        PropertyValue::Array(
            items
                .iter()
                .map(|s| PropertyValue::String(s.clone()))
                .collect(),
        )
    }

    fn pv_to_strings(node: &Node, key: &str) -> Vec<String> {
        match node.properties.get(key) {
            Some(PropertyValue::Array(arr)) => arr
                .iter()
                .filter_map(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        }
    }

    fn acl_node(
        id: &str,
        name: &str,
        path: &str,
        node_type: &str,
        properties: HashMap<String, PropertyValue>,
    ) -> Node {
        let parent = path
            .trim_end_matches('/')
            .rsplit('/')
            .nth(1)
            .map(|s| s.to_string());
        Node {
            id: id.to_string(),
            name: name.to_string(),
            path: path.to_string(),
            node_type: node_type.to_string(),
            archetype: None,
            properties,
            children: vec![],
            order_key: "a".to_string(),
            has_children: None,
            parent,
            version: 1,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            tenant_id: None,
            workspace: Some(ACL_WORKSPACE.to_string()),
            owner_id: None,
            relations: vec![],
        }
    }

    // =========================================================================
    // Role CRUD
    // =========================================================================

    async fn acl_create_role(&self, stmt: &CreateRole) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/roles/{}", stmt.role_id);

        if self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "Role '{}' already exists",
                stmt.role_id
            )));
        }

        let mut props = HashMap::new();
        props.insert(
            "role_id".to_string(),
            PropertyValue::String(stmt.role_id.clone()),
        );
        props.insert(
            "name".to_string(),
            PropertyValue::String(stmt.role_id.clone()),
        );
        if let Some(ref d) = stmt.description {
            props.insert("description".to_string(), PropertyValue::String(d.clone()));
        }
        if !stmt.inherits.is_empty() {
            props.insert("inherits".to_string(), Self::strings_to_pv(&stmt.inherits));
        }
        if !stmt.permissions.is_empty() {
            props.insert(
                "permissions".to_string(),
                PropertyValue::Array(stmt.permissions.iter().map(Self::perm_to_pv).collect()),
            );
        }

        let node = Self::acl_node(
            &nanoid::nanoid!(),
            &stmt.role_id,
            &path,
            "raisin:Role",
            props,
        );
        self.storage
            .nodes()
            .create(scope, node, Self::acl_create_opts())
            .await?;
        Self::acl_ok(format!("Role '{}' created", stmt.role_id))
    }

    async fn acl_alter_role(&self, stmt: &AlterRole) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/roles/{}", stmt.role_id);
        let mut node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Role '{}' not found", stmt.role_id)))?;

        match &stmt.action {
            AlterRoleAction::SetDescription(d) => {
                node.properties
                    .insert("description".to_string(), PropertyValue::String(d.clone()));
            }
            AlterRoleAction::AddInherits(parents) => {
                let mut cur = Self::pv_to_strings(&node, "inherits");
                for p in parents {
                    if !cur.contains(p) {
                        cur.push(p.clone());
                    }
                }
                node.properties
                    .insert("inherits".to_string(), Self::strings_to_pv(&cur));
            }
            AlterRoleAction::DropInherits(parents) => {
                let cur = Self::pv_to_strings(&node, "inherits");
                let f: Vec<String> = cur.into_iter().filter(|c| !parents.contains(c)).collect();
                node.properties
                    .insert("inherits".to_string(), Self::strings_to_pv(&f));
            }
            AlterRoleAction::AddPermission(grant) => {
                let mut perms = match node.properties.get("permissions") {
                    Some(PropertyValue::Array(arr)) => arr.clone(),
                    _ => vec![],
                };
                perms.push(Self::perm_to_pv(grant));
                node.properties
                    .insert("permissions".to_string(), PropertyValue::Array(perms));
            }
            AlterRoleAction::DropPermission(idx) => {
                let mut perms = match node.properties.get("permissions") {
                    Some(PropertyValue::Array(arr)) => arr.clone(),
                    _ => vec![],
                };
                if *idx < perms.len() {
                    perms.remove(*idx);
                }
                node.properties
                    .insert("permissions".to_string(), PropertyValue::Array(perms));
            }
        }

        self.storage
            .nodes()
            .update(scope, node, Self::acl_update_opts())
            .await?;
        Self::acl_ok(format!("Role '{}' altered", stmt.role_id))
    }

    async fn acl_drop_role(&self, stmt: &DropRole) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/roles/{}", stmt.role_id);
        let deleted = self
            .storage
            .nodes()
            .delete_by_path(scope, &path, DeleteNodeOptions::default())
            .await?;
        if !deleted && !stmt.if_exists {
            return Err(Error::NotFound(format!(
                "Role '{}' not found",
                stmt.role_id
            )));
        }
        Self::acl_ok(format!("Role '{}' dropped", stmt.role_id))
    }

    async fn acl_show_roles(&self, stmt: &ShowRoles) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let nodes = self
            .storage
            .nodes()
            .list_by_type(scope, "raisin:Role", ListOptions::for_sql())
            .await?;
        let rows: Vec<Row> = nodes
            .into_iter()
            .filter(|n| {
                stmt.like_pattern
                    .as_ref()
                    .map_or(true, |p| like_match(p, &n.name))
            })
            .map(|n| {
                let mut row = Row::new();
                row.insert("role_id".to_string(), PropertyValue::String(n.name.clone()));
                row.insert(
                    "description".to_string(),
                    n.properties
                        .get("description")
                        .cloned()
                        .unwrap_or(PropertyValue::Null),
                );
                row.insert(
                    "inherits".to_string(),
                    n.properties
                        .get("inherits")
                        .cloned()
                        .unwrap_or(PropertyValue::Null),
                );
                row
            })
            .collect();
        Self::acl_rows(rows)
    }

    async fn acl_describe_role(&self, stmt: &DescribeRole) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/roles/{}", stmt.role_id);
        let node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Role '{}' not found", stmt.role_id)))?;

        let mut rows = Vec::new();
        for (key, value) in &node.properties {
            let mut row = Row::new();
            row.insert("property".to_string(), PropertyValue::String(key.clone()));
            row.insert(
                "value".to_string(),
                PropertyValue::String(format!("{:?}", value)),
            );
            rows.push(row);
        }
        Self::acl_rows(rows)
    }

    // =========================================================================
    // Group CRUD
    // =========================================================================

    async fn acl_create_group(&self, stmt: &CreateGroup) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/groups/{}", stmt.group_id);

        if self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "Group '{}' already exists",
                stmt.group_id
            )));
        }

        let mut props = HashMap::new();
        props.insert(
            "group_id".to_string(),
            PropertyValue::String(stmt.group_id.clone()),
        );
        props.insert(
            "name".to_string(),
            PropertyValue::String(stmt.group_id.clone()),
        );
        if let Some(ref d) = stmt.description {
            props.insert("description".to_string(), PropertyValue::String(d.clone()));
        }
        if !stmt.roles.is_empty() {
            props.insert("roles".to_string(), Self::strings_to_pv(&stmt.roles));
        }

        let node = Self::acl_node(
            &nanoid::nanoid!(),
            &stmt.group_id,
            &path,
            "raisin:Group",
            props,
        );
        self.storage
            .nodes()
            .create(scope, node, Self::acl_create_opts())
            .await?;
        Self::acl_ok(format!("Group '{}' created", stmt.group_id))
    }

    async fn acl_alter_group(&self, stmt: &AlterGroup) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/groups/{}", stmt.group_id);
        let mut node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Group '{}' not found", stmt.group_id)))?;

        match &stmt.action {
            AlterGroupAction::SetDescription(d) => {
                node.properties
                    .insert("description".to_string(), PropertyValue::String(d.clone()));
            }
            AlterGroupAction::AddRoles(roles) => {
                let mut cur = Self::pv_to_strings(&node, "roles");
                for r in roles {
                    if !cur.contains(r) {
                        cur.push(r.clone());
                    }
                }
                node.properties
                    .insert("roles".to_string(), Self::strings_to_pv(&cur));
            }
            AlterGroupAction::DropRoles(roles) => {
                let cur = Self::pv_to_strings(&node, "roles");
                let f: Vec<String> = cur.into_iter().filter(|c| !roles.contains(c)).collect();
                node.properties
                    .insert("roles".to_string(), Self::strings_to_pv(&f));
            }
        }

        self.storage
            .nodes()
            .update(scope, node, Self::acl_update_opts())
            .await?;
        Self::acl_ok(format!("Group '{}' altered", stmt.group_id))
    }

    async fn acl_drop_group(&self, stmt: &DropGroup) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/groups/{}", stmt.group_id);
        let deleted = self
            .storage
            .nodes()
            .delete_by_path(scope, &path, DeleteNodeOptions::default())
            .await?;
        if !deleted && !stmt.if_exists {
            return Err(Error::NotFound(format!(
                "Group '{}' not found",
                stmt.group_id
            )));
        }
        Self::acl_ok(format!("Group '{}' dropped", stmt.group_id))
    }

    async fn acl_show_groups(&self, stmt: &ShowGroups) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let nodes = self
            .storage
            .nodes()
            .list_by_type(scope, "raisin:Group", ListOptions::for_sql())
            .await?;
        let rows: Vec<Row> = nodes
            .into_iter()
            .filter(|n| {
                stmt.like_pattern
                    .as_ref()
                    .map_or(true, |p| like_match(p, &n.name))
            })
            .map(|n| {
                let mut row = Row::new();
                row.insert(
                    "group_id".to_string(),
                    PropertyValue::String(n.name.clone()),
                );
                row.insert(
                    "description".to_string(),
                    n.properties
                        .get("description")
                        .cloned()
                        .unwrap_or(PropertyValue::Null),
                );
                row.insert(
                    "roles".to_string(),
                    n.properties
                        .get("roles")
                        .cloned()
                        .unwrap_or(PropertyValue::Null),
                );
                row
            })
            .collect();
        Self::acl_rows(rows)
    }

    async fn acl_describe_group(&self, stmt: &DescribeGroup) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/groups/{}", stmt.group_id);
        let node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Group '{}' not found", stmt.group_id)))?;

        let mut rows = Vec::new();
        for (key, value) in &node.properties {
            let mut row = Row::new();
            row.insert("property".to_string(), PropertyValue::String(key.clone()));
            row.insert(
                "value".to_string(),
                PropertyValue::String(format!("{:?}", value)),
            );
            rows.push(row);
        }
        Self::acl_rows(rows)
    }

    // =========================================================================
    // User CRUD
    // =========================================================================

    async fn acl_create_user(&self, stmt: &CreateUser) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = if let Some(ref folder) = stmt.folder {
            format!("{}/{}", folder.trim_end_matches('/'), stmt.user_id)
        } else {
            format!("/users/{}", stmt.user_id)
        };

        if self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .is_some()
        {
            return Err(Error::Conflict(format!(
                "User '{}' already exists",
                stmt.user_id
            )));
        }

        let mut props = HashMap::new();
        props.insert(
            "user_id".to_string(),
            PropertyValue::String(stmt.user_id.clone()),
        );
        props.insert(
            "email".to_string(),
            PropertyValue::String(stmt.email.clone()),
        );
        if let Some(ref dn) = stmt.display_name {
            props.insert(
                "display_name".to_string(),
                PropertyValue::String(dn.clone()),
            );
        }
        if !stmt.roles.is_empty() {
            props.insert("roles".to_string(), Self::strings_to_pv(&stmt.roles));
        }
        if !stmt.groups.is_empty() {
            props.insert("groups".to_string(), Self::strings_to_pv(&stmt.groups));
        }
        if let Some(can_login) = stmt.can_login {
            props.insert("can_login".to_string(), PropertyValue::Boolean(can_login));
        }
        if let Some(ref bd) = stmt.birth_date {
            props.insert("birth_date".to_string(), PropertyValue::String(bd.clone()));
        }

        let node = Self::acl_node(
            &nanoid::nanoid!(),
            &stmt.user_id,
            &path,
            "raisin:User",
            props,
        );
        self.storage
            .nodes()
            .create(scope, node, Self::acl_create_opts())
            .await?;
        Self::acl_ok(format!("User '{}' created", stmt.user_id))
    }

    async fn acl_alter_user(&self, stmt: &AlterUser) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/users/{}", stmt.user_id);
        let mut node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("User '{}' not found", stmt.user_id)))?;

        match &stmt.action {
            AlterUserAction::SetEmail(v) => {
                node.properties
                    .insert("email".to_string(), PropertyValue::String(v.clone()));
            }
            AlterUserAction::SetDisplayName(v) => {
                node.properties
                    .insert("display_name".to_string(), PropertyValue::String(v.clone()));
            }
            AlterUserAction::SetCanLogin(v) => {
                node.properties
                    .insert("can_login".to_string(), PropertyValue::Boolean(*v));
            }
            AlterUserAction::SetBirthDate(v) => {
                node.properties
                    .insert("birth_date".to_string(), PropertyValue::String(v.clone()));
            }
            AlterUserAction::AddRoles(roles) => {
                let mut cur = Self::pv_to_strings(&node, "roles");
                for r in roles {
                    if !cur.contains(r) {
                        cur.push(r.clone());
                    }
                }
                node.properties
                    .insert("roles".to_string(), Self::strings_to_pv(&cur));
            }
            AlterUserAction::DropRoles(roles) => {
                let cur = Self::pv_to_strings(&node, "roles");
                let f: Vec<String> = cur.into_iter().filter(|c| !roles.contains(c)).collect();
                node.properties
                    .insert("roles".to_string(), Self::strings_to_pv(&f));
            }
            AlterUserAction::AddGroups(groups) => {
                let mut cur = Self::pv_to_strings(&node, "groups");
                for g in groups {
                    if !cur.contains(g) {
                        cur.push(g.clone());
                    }
                }
                node.properties
                    .insert("groups".to_string(), Self::strings_to_pv(&cur));
            }
            AlterUserAction::DropGroups(groups) => {
                let cur = Self::pv_to_strings(&node, "groups");
                let f: Vec<String> = cur.into_iter().filter(|c| !groups.contains(c)).collect();
                node.properties
                    .insert("groups".to_string(), Self::strings_to_pv(&f));
            }
        }

        self.storage
            .nodes()
            .update(scope, node, Self::acl_update_opts())
            .await?;
        Self::acl_ok(format!("User '{}' altered", stmt.user_id))
    }

    async fn acl_drop_user(&self, stmt: &DropUser) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/users/{}", stmt.user_id);
        let deleted = self
            .storage
            .nodes()
            .delete_by_path(scope, &path, DeleteNodeOptions::default())
            .await?;
        if !deleted && !stmt.if_exists {
            return Err(Error::NotFound(format!(
                "User '{}' not found",
                stmt.user_id
            )));
        }
        Self::acl_ok(format!("User '{}' dropped", stmt.user_id))
    }

    async fn acl_show_users(&self, stmt: &ShowUsers) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let nodes = self
            .storage
            .nodes()
            .list_by_type(scope, "raisin:User", ListOptions::for_sql())
            .await?;
        let rows: Vec<Row> = nodes
            .into_iter()
            .filter(|n| {
                if let Some(ref p) = stmt.like_pattern {
                    if !like_match(p, &n.name) {
                        return false;
                    }
                }
                if let Some(ref g) = stmt.in_group {
                    if !Self::pv_to_strings(n, "groups").contains(g) {
                        return false;
                    }
                }
                if let Some(ref r) = stmt.with_role {
                    if !Self::pv_to_strings(n, "roles").contains(r) {
                        return false;
                    }
                }
                true
            })
            .map(|n| {
                let mut row = Row::new();
                row.insert("user_id".to_string(), PropertyValue::String(n.name.clone()));
                row.insert(
                    "email".to_string(),
                    n.properties
                        .get("email")
                        .cloned()
                        .unwrap_or(PropertyValue::Null),
                );
                row.insert(
                    "display_name".to_string(),
                    n.properties
                        .get("display_name")
                        .cloned()
                        .unwrap_or(PropertyValue::Null),
                );
                row.insert(
                    "roles".to_string(),
                    n.properties
                        .get("roles")
                        .cloned()
                        .unwrap_or(PropertyValue::Null),
                );
                row.insert(
                    "groups".to_string(),
                    n.properties
                        .get("groups")
                        .cloned()
                        .unwrap_or(PropertyValue::Null),
                );
                row
            })
            .collect();
        Self::acl_rows(rows)
    }

    async fn acl_describe_user(&self, stmt: &DescribeUser) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/users/{}", stmt.user_id);
        let node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("User '{}' not found", stmt.user_id)))?;

        let mut rows = Vec::new();
        for (key, value) in &node.properties {
            let mut row = Row::new();
            row.insert("property".to_string(), PropertyValue::String(key.clone()));
            row.insert(
                "value".to_string(),
                PropertyValue::String(format!("{:?}", value)),
            );
            rows.push(row);
        }
        Self::acl_rows(rows)
    }

    // =========================================================================
    // Grant / Revoke
    // =========================================================================

    async fn acl_grant(&self, stmt: &Grant) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let (path, label) = match &stmt.target {
            GrantTarget::User(id) => (format!("/users/{}", id), format!("User '{}'", id)),
            GrantTarget::Group(id) => (format!("/groups/{}", id), format!("Group '{}'", id)),
        };

        let mut node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("{} not found", label)))?;

        for item in &stmt.grants {
            match item {
                GrantItem::Role(role) => {
                    let mut roles = Self::pv_to_strings(&node, "roles");
                    if !roles.contains(role) {
                        roles.push(role.clone());
                    }
                    node.properties
                        .insert("roles".to_string(), Self::strings_to_pv(&roles));
                }
                GrantItem::Group(group) => {
                    let mut groups = Self::pv_to_strings(&node, "groups");
                    if !groups.contains(group) {
                        groups.push(group.clone());
                    }
                    node.properties
                        .insert("groups".to_string(), Self::strings_to_pv(&groups));
                }
            }
        }

        self.storage
            .nodes()
            .update(scope, node, Self::acl_update_opts())
            .await?;
        Self::acl_ok(format!("Grants applied to {}", label))
    }

    async fn acl_revoke(&self, stmt: &Revoke) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let (path, label) = match &stmt.target {
            RevokeTarget::User(id) => (format!("/users/{}", id), format!("User '{}'", id)),
            RevokeTarget::Group(id) => (format!("/groups/{}", id), format!("Group '{}'", id)),
        };

        let mut node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("{} not found", label)))?;

        for item in &stmt.revocations {
            match item {
                RevokeItem::Role(role) => {
                    let roles = Self::pv_to_strings(&node, "roles");
                    let f: Vec<String> = roles.into_iter().filter(|r| r != role).collect();
                    node.properties
                        .insert("roles".to_string(), Self::strings_to_pv(&f));
                }
                RevokeItem::Group(group) => {
                    let groups = Self::pv_to_strings(&node, "groups");
                    let f: Vec<String> = groups.into_iter().filter(|g| g != group).collect();
                    node.properties
                        .insert("groups".to_string(), Self::strings_to_pv(&f));
                }
            }
        }

        self.storage
            .nodes()
            .update(scope, node, Self::acl_update_opts())
            .await?;
        Self::acl_ok(format!("Revocations applied to {}", label))
    }

    // =========================================================================
    // Security Config
    // =========================================================================

    async fn acl_alter_security_config(
        &self,
        stmt: &AlterSecurityConfig,
    ) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/config/{}", stmt.workspace_pattern);

        let existing = self.storage.nodes().get_by_path(scope, &path, None).await?;
        let is_new = existing.is_none();

        let mut node = existing.unwrap_or_else(|| {
            let mut props = HashMap::new();
            props.insert(
                "workspace_pattern".to_string(),
                PropertyValue::String(stmt.workspace_pattern.clone()),
            );
            Self::acl_node(
                &nanoid::nanoid!(),
                &stmt.workspace_pattern,
                &path,
                "raisin:SecurityConfig",
                props,
            )
        });

        for setting in &stmt.settings {
            match setting {
                SecurityConfigSetting::DefaultPolicy(v) => {
                    node.properties.insert(
                        "default_policy".to_string(),
                        PropertyValue::String(v.clone()),
                    );
                }
                SecurityConfigSetting::AnonymousEnabled(v) => {
                    node.properties
                        .insert("anonymous_enabled".to_string(), PropertyValue::Boolean(*v));
                }
                SecurityConfigSetting::AnonymousRole(v) => {
                    node.properties.insert(
                        "anonymous_role".to_string(),
                        PropertyValue::String(v.clone()),
                    );
                }
                SecurityConfigSetting::InterfaceSetting {
                    interface,
                    key,
                    value,
                } => {
                    let prop_key = format!("{}.{}", interface, key);
                    node.properties
                        .insert(prop_key, PropertyValue::String(value.clone()));
                }
            }
        }

        if is_new {
            self.storage
                .nodes()
                .create(scope, node, Self::acl_create_opts())
                .await?;
        } else {
            self.storage
                .nodes()
                .update(scope, node, Self::acl_update_opts())
                .await?;
        }

        Self::acl_ok(format!(
            "Security config for '{}' updated",
            stmt.workspace_pattern
        ))
    }

    async fn acl_show_security_config(
        &self,
        stmt: &ShowSecurityConfig,
    ) -> Result<RowStream, Error> {
        let scope = self.acl_scope();

        if let Some(ref ws) = stmt.workspace {
            let path = format!("/config/{}", ws);
            match self.storage.nodes().get_by_path(scope, &path, None).await? {
                Some(n) => {
                    let rows: Vec<Row> = n
                        .properties
                        .iter()
                        .map(|(k, v)| {
                            let mut row = Row::new();
                            row.insert(
                                "workspace_pattern".to_string(),
                                PropertyValue::String(ws.clone()),
                            );
                            row.insert("setting".to_string(), PropertyValue::String(k.clone()));
                            row.insert(
                                "value".to_string(),
                                PropertyValue::String(format!("{:?}", v)),
                            );
                            row
                        })
                        .collect();
                    Self::acl_rows(rows)
                }
                None => Self::acl_rows(vec![]),
            }
        } else {
            let nodes = self
                .storage
                .nodes()
                .list_by_type(scope, "raisin:SecurityConfig", ListOptions::for_sql())
                .await?;
            let rows: Vec<Row> = nodes
                .into_iter()
                .map(|n| {
                    let mut row = Row::new();
                    row.insert(
                        "workspace_pattern".to_string(),
                        n.properties
                            .get("workspace_pattern")
                            .cloned()
                            .unwrap_or_else(|| PropertyValue::String(n.name.clone())),
                    );
                    row.insert(
                        "default_policy".to_string(),
                        n.properties
                            .get("default_policy")
                            .cloned()
                            .unwrap_or(PropertyValue::Null),
                    );
                    row.insert(
                        "anonymous_enabled".to_string(),
                        n.properties
                            .get("anonymous_enabled")
                            .cloned()
                            .unwrap_or(PropertyValue::Null),
                    );
                    row
                })
                .collect();
            Self::acl_rows(rows)
        }
    }

    // =========================================================================
    // Introspection
    // =========================================================================

    async fn acl_show_permissions_for(
        &self,
        stmt: &ShowPermissionsFor,
    ) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/users/{}", stmt.user_id);
        let user_node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("User '{}' not found", stmt.user_id)))?;

        // Collect all roles (direct + from groups)
        let mut all_roles = Self::pv_to_strings(&user_node, "roles");
        for gid in &Self::pv_to_strings(&user_node, "groups") {
            let gpath = format!("/groups/{}", gid);
            if let Some(gn) = self
                .storage
                .nodes()
                .get_by_path(scope, &gpath, None)
                .await?
            {
                for r in Self::pv_to_strings(&gn, "roles") {
                    if !all_roles.contains(&r) {
                        all_roles.push(r);
                    }
                }
            }
        }

        // Collect permissions from each role
        let mut rows = Vec::new();
        for role_id in &all_roles {
            let rpath = format!("/roles/{}", role_id);
            if let Some(rn) = self
                .storage
                .nodes()
                .get_by_path(scope, &rpath, None)
                .await?
            {
                if let Some(PropertyValue::Array(perms)) = rn.properties.get("permissions") {
                    for perm in perms {
                        if let PropertyValue::Object(obj) = perm {
                            // Optionally filter by workspace
                            if let Some(ref ws_filter) = stmt.workspace {
                                if let Some(PropertyValue::String(ws)) = obj.get("workspace") {
                                    if ws != ws_filter {
                                        continue;
                                    }
                                }
                            }
                            let mut row = Row::new();
                            row.insert("role".to_string(), PropertyValue::String(role_id.clone()));
                            row.insert(
                                "path".to_string(),
                                obj.get("path")
                                    .cloned()
                                    .unwrap_or(PropertyValue::String("*".to_string())),
                            );
                            row.insert(
                                "operations".to_string(),
                                obj.get("operations")
                                    .cloned()
                                    .unwrap_or(PropertyValue::Null),
                            );
                            row.insert(
                                "workspace".to_string(),
                                obj.get("workspace").cloned().unwrap_or(PropertyValue::Null),
                            );
                            rows.push(row);
                        }
                    }
                }
            }
        }
        Self::acl_rows(rows)
    }

    async fn acl_show_effective_roles_for(
        &self,
        stmt: &ShowEffectiveRolesFor,
    ) -> Result<RowStream, Error> {
        let scope = self.acl_scope();
        let path = format!("/users/{}", stmt.user_id);
        let user_node = self
            .storage
            .nodes()
            .get_by_path(scope, &path, None)
            .await?
            .ok_or_else(|| Error::NotFound(format!("User '{}' not found", stmt.user_id)))?;

        let direct_roles = Self::pv_to_strings(&user_node, "roles");
        let user_groups = Self::pv_to_strings(&user_node, "groups");

        // Roles from groups
        let mut group_roles: Vec<(String, String)> = Vec::new();
        for gid in &user_groups {
            let gpath = format!("/groups/{}", gid);
            if let Some(gn) = self
                .storage
                .nodes()
                .get_by_path(scope, &gpath, None)
                .await?
            {
                for r in Self::pv_to_strings(&gn, "roles") {
                    group_roles.push((r, gid.clone()));
                }
            }
        }

        // Inherited roles (walk INHERITS chains)
        let mut inherited: Vec<(String, String)> = Vec::new();
        let mut to_visit: Vec<String> = direct_roles.clone();
        for (r, _) in &group_roles {
            if !to_visit.contains(r) {
                to_visit.push(r.clone());
            }
        }
        let mut visited: Vec<String> = Vec::new();

        while let Some(rid) = to_visit.pop() {
            if visited.contains(&rid) {
                continue;
            }
            visited.push(rid.clone());
            let rpath = format!("/roles/{}", rid);
            if let Some(rn) = self
                .storage
                .nodes()
                .get_by_path(scope, &rpath, None)
                .await?
            {
                for parent in Self::pv_to_strings(&rn, "inherits") {
                    inherited.push((parent.clone(), rid.clone()));
                    if !visited.contains(&parent) {
                        to_visit.push(parent);
                    }
                }
            }
        }

        let mut rows = Vec::new();
        for role in &direct_roles {
            let mut row = Row::new();
            row.insert("role".to_string(), PropertyValue::String(role.clone()));
            row.insert(
                "source".to_string(),
                PropertyValue::String("direct".to_string()),
            );
            row.insert("via".to_string(), PropertyValue::Null);
            rows.push(row);
        }
        for (role, group) in &group_roles {
            if !direct_roles.contains(role) {
                let mut row = Row::new();
                row.insert("role".to_string(), PropertyValue::String(role.clone()));
                row.insert(
                    "source".to_string(),
                    PropertyValue::String("group".to_string()),
                );
                row.insert("via".to_string(), PropertyValue::String(group.clone()));
                rows.push(row);
            }
        }
        for (role, parent) in &inherited {
            let mut row = Row::new();
            row.insert("role".to_string(), PropertyValue::String(role.clone()));
            row.insert(
                "source".to_string(),
                PropertyValue::String("inherited".to_string()),
            );
            row.insert("via".to_string(), PropertyValue::String(parent.clone()));
            rows.push(row);
        }
        Self::acl_rows(rows)
    }
}

// =============================================================================
// Utility
// =============================================================================

/// Simple SQL LIKE pattern matching (supports `%` and `_` wildcards).
fn like_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let value = value.to_lowercase();
    like_match_inner(pattern.as_bytes(), value.as_bytes())
}

fn like_match_inner(pattern: &[u8], value: &[u8]) -> bool {
    match (pattern.first(), value.first()) {
        (None, None) => true,
        (Some(b'%'), _) => {
            like_match_inner(&pattern[1..], value)
                || (!value.is_empty() && like_match_inner(pattern, &value[1..]))
        }
        (Some(b'_'), Some(_)) => like_match_inner(&pattern[1..], &value[1..]),
        (Some(a), Some(b)) if a == b => like_match_inner(&pattern[1..], &value[1..]),
        _ => false,
    }
}
