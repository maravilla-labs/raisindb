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

//! Caller identity, scopes, and tenant scoping for MCP sessions.
//!
//! Every tool invocation carries an [`McpIdentity`] describing who is calling,
//! the scopes they were granted, and which tenant / workspace / branch the call
//! resolves against. In a managed deployment the tenant is pinned from the
//! connection; in single-tenant mode it defaults to the configured workspace.
//!
//! [`McpIdentity`] also bridges to the core [`raisin_models::auth::AuthContext`]
//! so RLS is enforced by the same identity that authorized the MCP call.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use raisin_models::auth::AuthContext;

/// Default workspace used when a request does not name one.
pub const DEFAULT_WORKSPACE: &str = "default";

/// Default branch used when a session does not pin one.
pub const DEFAULT_BRANCH: &str = "main";

/// Deployment mode the MCP server runs in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantMode {
    /// Single-tenant: all calls resolve against one fixed workspace.
    #[default]
    Single,
    /// Managed deployment: each session is pinned to a tenant id.
    Managed,
}

/// Identity, scopes, and scope context attached to an MCP session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpIdentity {
    /// Opaque subject identifier of the caller (user / service / api-key id).
    ///
    /// `None` marks an anonymous caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Tenant the session is pinned to, when running in managed mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// Repository the calls resolve against.
    pub repo: String,
    /// Workspace the calls resolve against.
    pub workspace: String,
    /// Branch within the workspace.
    pub branch: String,
    /// Scopes granted to this identity (used to gate tool access).
    #[serde(default)]
    pub scopes: BTreeSet<String>,
    /// Whether this identity bypasses row-level security (system caller).
    #[serde(default)]
    pub is_system: bool,
    /// Roles carried through to RLS evaluation.
    #[serde(default)]
    pub roles: Vec<String>,
}

impl McpIdentity {
    /// Construct an authenticated identity for `subject` against `repo`.
    pub fn new(subject: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            subject: Some(subject.into()),
            tenant_id: None,
            repo: repo.into(),
            workspace: DEFAULT_WORKSPACE.to_string(),
            branch: DEFAULT_BRANCH.to_string(),
            scopes: BTreeSet::new(),
            is_system: false,
            roles: Vec::new(),
        }
    }

    /// Construct an anonymous identity against `repo`.
    pub fn anonymous(repo: impl Into<String>) -> Self {
        Self {
            subject: None,
            tenant_id: None,
            repo: repo.into(),
            workspace: DEFAULT_WORKSPACE.to_string(),
            branch: DEFAULT_BRANCH.to_string(),
            scopes: BTreeSet::new(),
            is_system: false,
            roles: Vec::new(),
        }
    }

    /// Returns `true` when no subject is attached.
    pub fn is_anonymous(&self) -> bool {
        self.subject.is_none()
    }

    /// Pin this identity to a tenant (managed deployments).
    pub fn with_tenant(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Target a specific workspace.
    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = workspace.into();
        self
    }

    /// Target a specific branch within the workspace.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = branch.into();
        self
    }

    /// Grant a set of scopes to this identity.
    pub fn with_scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    /// Mark this identity as a system caller (bypasses RLS).
    pub fn as_system(mut self) -> Self {
        self.is_system = true;
        self
    }

    /// Attach roles carried through to RLS evaluation.
    pub fn with_roles<I, S>(mut self, roles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.roles = roles.into_iter().map(Into::into).collect();
        self
    }

    /// Whether this identity holds every scope in `required`.
    ///
    /// A system identity satisfies any scope requirement. An empty requirement
    /// is always satisfied.
    pub fn has_scopes<S: AsRef<str>>(&self, required: &[S]) -> bool {
        if self.is_system {
            return true;
        }
        required
            .iter()
            .all(|scope| self.scopes.contains(scope.as_ref()))
    }

    /// The scopes in `required` that this identity is missing.
    pub fn missing_scopes<S: AsRef<str>>(&self, required: &[S]) -> Vec<String> {
        if self.is_system {
            return Vec::new();
        }
        required
            .iter()
            .filter(|scope| !self.scopes.contains(scope.as_ref()))
            .map(|s| s.as_ref().to_string())
            .collect()
    }

    /// Build the core [`AuthContext`] this identity maps onto for RLS.
    ///
    /// System identities map to [`AuthContext::system`]; anonymous identities to
    /// [`AuthContext::anonymous`]; otherwise to [`AuthContext::for_user`] with
    /// the carried roles applied.
    pub fn to_auth_context(&self) -> AuthContext {
        if self.is_system {
            return AuthContext::system();
        }
        match &self.subject {
            None => AuthContext::anonymous(),
            Some(subject) => {
                let mut ctx = AuthContext::for_user(subject.clone());
                ctx.roles = self.roles.clone();
                ctx
            }
        }
    }
}

impl Default for McpIdentity {
    fn default() -> Self {
        Self::anonymous(crate::registry::DEFAULT_REPO)
    }
}
