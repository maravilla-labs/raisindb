// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `SHOW / ALTER / REBUILD / VERIFY SPATIAL INDEX` — the operator surface for the
//! geospatial index, expressed as SQL.
//!
//! Modelled on [`crate::ast::ai_config`] (`REBUILD VECTOR INDEX`,
//! `SHOW VECTOR INDEX HEALTH`) rather than on the NODETYPE DDL parser: these are
//! imperative admin commands, not schema definitions, and the vector equivalents
//! already established the shape operators know.
//!
//! # Grammar
//!
//! ```text
//! -- read-only (any authenticated caller)
//! SHOW SPATIAL INDEX CONFIG [ FOR 'workspace' [ PROPERTY 'prop' ] ]
//! SHOW SPATIAL INDEX HEALTH [ FOR 'workspace' [ PROPERTY 'prop' ] ]
//!
//! -- mutating (system_admin)
//! ALTER SPATIAL INDEX FOR 'workspace' [ PROPERTY 'prop' ]
//!       SET PRECISIONS = ( 11, 10, 9, 8, 7, 6, 4, 2 )
//!     [ SET SRID = 4326 ]
//!     [ SET BUCKET PROPERTY = 'floor' ]
//!     [ SET COVER = { CENTROID | EXTENT } ]
//! ALTER SPATIAL INDEX FOR 'workspace' [ PROPERTY 'prop' ] RESET
//! REBUILD SPATIAL INDEX FOR 'workspace' [ PROPERTY 'prop' ]
//! VERIFY  SPATIAL INDEX FOR 'workspace' [ PROPERTY 'prop' ]
//! ```
//!
//! # Why ALTER writes the WORKSPACE record and not a NodeType
//!
//! A bare property name cannot identify a NodeType, and a workspace may hold
//! several NodeTypes carrying the same geometry property. So `ALTER` writes
//! `WorkspaceConfig.spatial` — with `PROPERTY` it sets `spatial.properties[prop]`,
//! without it `spatial.default`. That is also what makes the change fan out across
//! a masterless cluster with nothing to broadcast: the workspace record is
//! replicated data, so every peer sees the new policy and reconciles its own local
//! index.
//!
//! Schema-level declaration (`location Geometry SPATIAL_INDEX (11, 10, …)` inside
//! `CREATE NODETYPE`) is the separate, higher-precedence path and is not part of
//! this statement family.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Which cells a non-point geometry is indexed under.
///
/// Mirrors `raisin_models::nodes::properties::SpatialCoverMode`; kept as a local
/// enum so `raisin-sql` does not depend on the models crate for a parser type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverModeSpec {
    /// Index the centroid only. The default and the historical behaviour.
    Centroid,
    /// Also index the cells covering the bounding box. Better selectivity, more
    /// write cost — this is the one setting that can push the write budget past
    /// the approved 2×, which is why it is opt-in.
    Extent,
}

impl fmt::Display for CoverModeSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoverModeSpec::Centroid => write!(f, "CENTROID"),
            CoverModeSpec::Extent => write!(f, "EXTENT"),
        }
    }
}

/// The fields an `ALTER SPATIAL INDEX` sets. `None` leaves the field untouched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpatialIndexSettings {
    /// Geohash precisions, as written. Canonicalised (clamped, deduped, sorted
    /// finest-first) by the model layer, so two operators writing the same set in
    /// a different order produce the same `policy_hash`.
    pub precisions: Option<Vec<usize>>,
    /// Default SRID for values of this property that carry no explicit `srid`.
    pub srid: Option<u32>,
    /// Sibling property whose value discriminates candidates inside a cell — a
    /// building floor label being the motivating case.
    pub bucket_property: Option<String>,
    /// Centroid-only or extent cover.
    pub cover: Option<CoverModeSpec>,
}

impl SpatialIndexSettings {
    /// Whether any field was set. An `ALTER` with none is a parse error, because
    /// silently succeeding while changing nothing is indistinguishable from a typo.
    pub fn is_empty(&self) -> bool {
        self.precisions.is_none()
            && self.srid.is_none()
            && self.bucket_property.is_none()
            && self.cover.is_none()
    }
}

/// A parsed spatial-index admin statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpatialAdminStatement {
    /// Report the *configured* policy (replicated intent).
    ShowConfig {
        workspace: Option<String>,
        property: Option<String>,
    },
    /// Report the *local* index state and a physical entry census (reality).
    ShowHealth {
        workspace: Option<String>,
        property: Option<String>,
    },
    /// Write `WorkspaceConfig.spatial`.
    Alter {
        workspace: String,
        property: Option<String>,
        settings: SpatialIndexSettings,
    },
    /// Remove the configured override, reverting to the next scope in the
    /// precedence chain (workspace default, then the server constants).
    Reset {
        workspace: String,
        property: Option<String>,
    },
    /// Queue a LOCAL rebuild. Not cluster-wide — see the module docs.
    Rebuild {
        workspace: String,
        property: Option<String>,
    },
    /// Compare configured intent against local reality and report the delta.
    Verify {
        workspace: String,
        property: Option<String>,
    },
}

impl SpatialAdminStatement {
    /// Human-readable operation name, used in log lines and in the authorization
    /// error message.
    pub fn operation(&self) -> &'static str {
        match self {
            SpatialAdminStatement::ShowConfig { .. } => "SHOW SPATIAL INDEX CONFIG",
            SpatialAdminStatement::ShowHealth { .. } => "SHOW SPATIAL INDEX HEALTH",
            SpatialAdminStatement::Alter { .. } => "ALTER SPATIAL INDEX",
            SpatialAdminStatement::Reset { .. } => "ALTER SPATIAL INDEX RESET",
            SpatialAdminStatement::Rebuild { .. } => "REBUILD SPATIAL INDEX",
            SpatialAdminStatement::Verify { .. } => "VERIFY SPATIAL INDEX",
        }
    }

    /// Whether this statement only reads.
    ///
    /// `VERIFY` is read-only in the sense that matters here — it writes nothing —
    /// but it is deliberately NOT in this set: it performs an unbounded scan of the
    /// index, which is an operator action, not something an anonymous reader should
    /// be able to trigger.
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            SpatialAdminStatement::ShowConfig { .. } | SpatialAdminStatement::ShowHealth { .. }
        )
    }

    /// The workspace this statement targets, if it names one.
    pub fn workspace(&self) -> Option<&str> {
        match self {
            SpatialAdminStatement::ShowConfig { workspace, .. }
            | SpatialAdminStatement::ShowHealth { workspace, .. } => workspace.as_deref(),
            SpatialAdminStatement::Alter { workspace, .. }
            | SpatialAdminStatement::Reset { workspace, .. }
            | SpatialAdminStatement::Rebuild { workspace, .. }
            | SpatialAdminStatement::Verify { workspace, .. } => Some(workspace),
        }
    }

    /// The property this statement targets, if it names one.
    pub fn property(&self) -> Option<&str> {
        match self {
            SpatialAdminStatement::ShowConfig { property, .. }
            | SpatialAdminStatement::ShowHealth { property, .. }
            | SpatialAdminStatement::Alter { property, .. }
            | SpatialAdminStatement::Reset { property, .. }
            | SpatialAdminStatement::Rebuild { property, .. }
            | SpatialAdminStatement::Verify { property, .. } => property.as_deref(),
        }
    }
}

impl fmt::Display for SpatialAdminStatement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.operation())?;
        if let Some(ws) = self.workspace() {
            write!(f, " FOR '{}'", ws)?;
        }
        if let Some(prop) = self.property() {
            write!(f, " PROPERTY '{}'", prop)?;
        }
        if let SpatialAdminStatement::Alter { settings, .. } = self {
            if let Some(p) = &settings.precisions {
                let list: Vec<String> = p.iter().map(|v| v.to_string()).collect();
                write!(f, " SET PRECISIONS = ({})", list.join(", "))?;
            }
            if let Some(s) = settings.srid {
                write!(f, " SET SRID = {}", s)?;
            }
            if let Some(b) = &settings.bucket_property {
                write!(f, " SET BUCKET PROPERTY = '{}'", b)?;
            }
            if let Some(c) = settings.cover {
                write!(f, " SET COVER = {}", c)?;
            }
        }
        Ok(())
    }
}
