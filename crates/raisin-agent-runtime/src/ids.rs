// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Identifiers, scope, subject and principal.
//!
//! Every id that enters a storage key is validated NUL-free and at most
//! [`MAX_KEY_PART`] bytes, because keys are `\0`-separated.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Longest id accepted into a storage key.
pub const MAX_KEY_PART: usize = 256;

/// An id is unusable in a storage key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid key part '{value}': {why}")]
pub struct InvalidKey {
    /// The offending value (truncated).
    pub value: String,
    /// Why.
    pub why: &'static str,
}

/// Check that `s` can be one `\0`-separated key segment.
pub fn validate_key_part(s: &str) -> Result<(), InvalidKey> {
    let bad = |why| {
        Err(InvalidKey {
            value: s.chars().take(64).collect(),
            why,
        })
    };
    if s.is_empty() {
        return bad("empty");
    }
    if s.len() > MAX_KEY_PART {
        return bad("longer than 256 bytes");
    }
    if s.as_bytes().contains(&0) {
        return bad("contains NUL");
    }
    Ok(())
}

macro_rules! string_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// Borrow as `&str`.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    };
}

string_id!(
    /// A run id: a v4 uuid. Ordering between runs is never by id.
    RunId
);
string_id!(
    /// `"{run_id}/op/{n}"`; also the operation's idempotency key.
    OperationId
);
string_id!(
    /// Client-supplied idempotency key of a control command.
    ControlId
);
string_id!(
    /// `"{run_id}/req/{n}"`.
    RequestId
);
string_id!(
    /// `"{run_id}/steer/{n}"`.
    SteerId
);
string_id!(
    /// A provider tool-call id, opaque.
    CallId
);

impl RunId {
    /// A fresh random run id.
    pub fn new_v4() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl OperationId {
    /// The `n`th operation of `run`.
    pub fn nth(run: &RunId, n: u64) -> Self {
        Self(format!("{run}/op/{n}"))
    }
}

impl RequestId {
    /// The `n`th request of `run`.
    pub fn nth(run: &RunId, n: u64) -> Self {
        Self(format!("{run}/req/{n}"))
    }
}

impl SteerId {
    /// The `n`th steer of `run`.
    pub fn nth(run: &RunId, n: u64) -> Self {
        Self(format!("{run}/steer/{n}"))
    }
}

macro_rules! num_id {
    ($(#[$m:meta])* $name:ident($t:ty)) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub $t);

        impl $name {
            /// The next value.
            pub fn next(self) -> Self {
                Self(self.0 + 1)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

num_id!(
    /// 1-based turn number, per run.
    TurnNo(u32)
);
num_id!(
    /// Event sequence number: 1-based, contiguous per run.
    Seq(u64)
);
num_id!(
    /// Record version, +1 per commit.
    Version(u64)
);
num_id!(
    /// Fencing token: +1 on every lease acquisition AND every lease clear.
    LeaseEpoch(u64)
);

/// Where a run executes. The branch is the execution context, not identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunScope {
    /// Tenant.
    pub tenant_id: String,
    /// Repository.
    pub repo_id: String,
    /// Branch.
    pub branch: String,
}

impl RunScope {
    /// Build a scope.
    pub fn new(tenant: &str, repo: &str, branch: &str) -> Self {
        Self {
            tenant_id: tenant.into(),
            repo_id: repo.into(),
            branch: branch.into(),
        }
    }

    /// Validate the parts that enter keys.
    pub fn validate(&self) -> Result<(), InvalidKey> {
        validate_key_part(&self.tenant_id)?;
        validate_key_part(&self.repo_id)?;
        validate_key_part(&self.branch)
    }
}

/// Opaque locator of whatever a run is "about". Core never interprets it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubjectRef {
    /// Workspace.
    pub workspace: String,
    /// Path.
    pub path: String,
    /// Node id, preferred for identity when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

impl SubjectRef {
    /// `"{workspace}\u{1f}{node_id or path}"`, validated NUL-free.
    pub fn key(&self) -> Result<String, InvalidKey> {
        let key = format!(
            "{}\u{1f}{}",
            self.workspace,
            self.node_id.as_deref().unwrap_or(&self.path)
        );
        validate_key_part(&key)?;
        Ok(key)
    }
}

/// Who a run executes as, or who issues a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    /// A human user.
    User,
    /// An agent identity.
    Agent,
    /// The system itself; requires a [`SystemToken`].
    System,
}

/// The identity a run executes under. Persisted at create, immutable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    /// Kind.
    pub kind: PrincipalKind,
    /// Id.
    pub id: String,
    /// The user an agent acts for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<String>,
}

impl Principal {
    /// A user principal.
    pub fn user(id: &str) -> Self {
        Self {
            kind: PrincipalKind::User,
            id: id.into(),
            on_behalf_of: None,
        }
    }
}

/// Proof that a caller is in-process system code.
///
/// It is not `Deserialize` and carries a private field, so no transport can
/// produce one from a request. Only server wiring should call
/// [`SystemToken::in_process`]; never hand one to code that acts on a request
/// body.
#[derive(Debug, Clone, Copy)]
pub struct SystemToken(());

impl SystemToken {
    /// Mint a token. In-process system callers only.
    pub fn in_process() -> Self {
        Self(())
    }
}
