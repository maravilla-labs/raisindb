//! Resolving what a run needs before it can start: the integration node, the
//! adapter path, the credential, the materialization scope, and the capability
//! cache.

use super::*;

impl VirtualMountSyncHandler {
    /// Decrypt the mount's account tokens into an adapter credential, stripping
    /// the refresh token. Returns `None` if no key/account/tokens are available.
    fn build_credential(
        &self,
        mount: &MountConfig,
        integration: &IntegrationConfig,
    ) -> Option<Value> {
        adapter::build_mount_credential(mount, integration)
    }

    /// Names of the connector's per-connection config fields flagged
    /// `meta.credential: true` — those that belong in the adapter credential
    /// rather than the mount config.
    ///
    /// Resolved through [`NodeTypeResolver`] so `extends`/mixins are honoured.
    /// Returns empty when the connector declares no `connection_config_type`
    /// (every pre-v2 connector), which is exactly the previous behaviour.
    async fn resolve_credential_fields(
        &self,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        integration: &IntegrationConfig,
    ) -> Vec<String> {
        use raisin_core::services::node_type_resolver::NodeTypeResolver;

        let Some(type_name) = integration.connection_config_type.as_deref() else {
            return Vec::new();
        };
        let resolver = NodeTypeResolver::new(
            self.storage.clone(),
            tenant.to_string(),
            repo.to_string(),
            // Config types live alongside the connector, on the config branch.
            config_branch.to_string(),
        );
        match resolver.resolve(type_name).await {
            Ok(resolved) => resolved
                .resolved_properties
                .iter()
                .filter(|p| {
                    p.meta
                        .as_ref()
                        .and_then(|m| m.get("credential"))
                        .is_some_and(|v| {
                            matches!(
                                v,
                                raisin_models::nodes::properties::PropertyValue::Boolean(true)
                            )
                        })
                })
                .filter_map(|p| p.name.clone())
                .collect(),
            Err(e) => {
                // A connector referencing a type its package never installed is
                // a misconfiguration, not a reason to stop syncing: the mount
                // still works, minus the credential-flagged fields.
                tracing::warn!(
                    config_type = %type_name,
                    error = %e,
                    "could not resolve connection config type; credential fields unavailable"
                );
                Vec::new()
            }
        }
    }

    /// Load a mount's backing `raisin:Integration` node.
    ///
    /// `integration_ref` may be either a path or a node id, so both are tried.
    /// Shared by the pre-flight connection check and [`Self::build_ctx_parts`]
    /// so the two can never disagree about which connector a mount points at.
    pub(super) async fn load_integration_node(
        &self,
        svc: &NodeService<RocksDBStorage>,
        mount: &MountConfig,
    ) -> Result<Option<raisin_models::nodes::Node>> {
        Ok(svc
            .get_by_path(&mount.integration_ref)
            .await?
            .or(svc.get(&mount.integration_ref).await.unwrap_or_default()))
    }

    /// Resolve everything needed to invoke a mount's adapter: the integration
    /// node id (for capability caching), adapter path, decrypted credential,
    /// mount snapshot, and materialization scope. Shared by the sync run, the
    /// disabled-mount push teardown, and the subscription-renew scan so all
    /// adapter invocations are built identically.
    pub(super) async fn build_ctx_parts(
        &self,
        svc: &NodeService<RocksDBStorage>,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        mount: &MountConfig,
    ) -> Result<CtxParts> {
        let Some(integ_node) = self.load_integration_node(svc, mount).await? else {
            return Err(Error::Validation(format!(
                "integration not found: {}",
                mount.integration_ref
            )));
        };
        let mut integration = IntegrationConfig::from_node(&integ_node)
            .map_err(|e| Error::Validation(format!("invalid integration: {e}")))?;
        // Resolve which per-connection config fields belong in the credential
        // rather than the mount config (an IMAP `username`). This needs storage,
        // so it cannot live in the synchronous `from_node` parse; doing it here
        // keeps that parse callable from the replication apply path.
        integration.credential_fields = self
            .resolve_credential_fields(tenant, repo, config_branch, &integration)
            .await;
        let adapter_path = mount
            .adapter_function
            .clone()
            .or_else(|| integration.adapter_function.clone())
            .ok_or_else(|| Error::Validation("no adapter_function configured".to_string()))?;
        let credential = self.build_credential(mount, &integration);
        // The snapshot carries the merged config, so it needs the chosen
        // connection. A selection error is not fatal here — the mount still gets
        // a snapshot (with no per-connection layer) and the missing credential
        // is what surfaces the problem, with `finalize` recording the reason.
        let account = integration.account_for(mount.account_ref.as_deref()).ok();
        let mount_snapshot = build_mount_snapshot(mount, &integration, account);
        let scope = MountScope {
            tenant: tenant.to_string(),
            repo: repo.to_string(),
            branch: mount.target_branch.clone(),
            workspace: mount.target_workspace.clone(),
            mount_id: mount.mount_id.clone(),
            mount_path: mount.mount_path.clone(),
            // Off for every ordinary path. `run_sync` turns it on for a remap;
            // the teardown and renew callers must never rewrite content.
            force_rewrite: false,
            watched_fields: mount.write_config.declared_mutable_fields().to_vec(),
            command_node_types: mount.write_config.command_node_types.clone(),
            // Pure config parsing, safe to resolve before capabilities are
            // probed — the same rationale as `watched_fields` above: what the
            // read path PRESERVES must not depend on what the adapter happens
            // to answer this run. A refusal (typo) reads as `false`: the write
            // plan already surfaces the bad config loudly, and the read path
            // falling back to today's remote-wins behaviour is the
            // conservative half.
            //
            // EVERY policy that KEEPS the local edit, not only `local_wins`.
            //
            // Under `error` the drain parks the conflict for a person to decide
            // — and then the same run's read phase rebuilt the node from the
            // provider and reseeded `__pushed_state`, so by the time anyone
            // looked there was nothing left to decide. The edit the mount had
            // just promised to hold was gone, and the park message described a
            // choice that no longer existed. A resolver policy had the same
            // hole: an intent parked because the resolver threw, declined, or
            // answered "ask a human" was reverted underneath it.
            //
            // `remote_wins` stays false, because there the revert IS the policy.
            read_local_wins: matches!(
                write::conflict::resolve_conflict_policy(mount),
                Ok(write::conflict::ConflictPolicy::LocalWins
                    | write::conflict::ConflictPolicy::Error
                    | write::conflict::ConflictPolicy::Resolver(_))
            ),
        };
        Ok(CtxParts {
            public_origin: integration.public_origin.clone(),
            integ_node_id: integ_node.id,
            adapter_path,
            credential,
            mount_snapshot,
            scope,
        })
    }

    /// Cache the resolved capabilities onto the integration node (on the config
    /// branch) so the admin UI can read them without invoking the adapter.
    pub(super) async fn cache_capabilities(
        &self,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        integration_id: &str,
        capabilities: &Capabilities,
    ) -> Result<()> {
        use raisin_models::nodes::properties::PropertyValue;

        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant, repo)?;
        tx.set_branch(config_branch)?;
        tx.set_actor(SYNC_ACTOR)?;
        // System privileges, sync identity: the auth context (not the raw
        // actor) is what stamps `updated_by`, so an `AuthContext::system()`
        // here would attribute the integration node to `"system"`.
        tx.set_auth_context(AuthContext::system_as(SYNC_ACTOR))?;
        tx.set_message("virtual mount sync: cache capabilities")?;
        // A capability cache is engine bookkeeping, same as mount state.
        tx.set_bookkeeping(true)?;

        let Some(mut node) = tx.get_node(SYSTEM_WORKSPACE, integration_id).await? else {
            return Ok(());
        };
        let caps_value = serde_json::to_value(capabilities)
            .map_err(|e| Error::Validation(format!("capabilities serialize failed: {e}")))?;

        // Only write when the answer CHANGED. Every sync run resolves
        // capabilities, and re-stamping an identical answer plus a fresh
        // `capabilities_checked_at` minted a replicated integration-node
        // revision per run — a third commit on top of the two mount-state
        // commits, for a value that almost never changes. The timestamp
        // therefore means "when this cached answer was established", which is
        // what the console needs it for (staleness display), not "the last
        // time a sync ran".
        let unchanged = node
            .properties
            .get("capabilities")
            .and_then(|pv| serde_json::to_value(pv).ok())
            .map(|stored| stored == caps_value)
            .unwrap_or(false);
        if unchanged {
            return Ok(());
        }

        let caps_pv: PropertyValue = serde_json::from_value(caps_value)
            .map_err(|e| Error::Validation(format!("capabilities to PropertyValue failed: {e}")))?;
        node.properties.insert("capabilities".to_string(), caps_pv);
        node.properties.insert(
            "capabilities_checked_at".to_string(),
            PropertyValue::String(Utc::now().to_rfc3339()),
        );
        tx.upsert_node(SYSTEM_WORKSPACE, &node).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) fn system_service(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> NodeService<RocksDBStorage> {
        NodeService::new_with_context(
            self.storage.clone(),
            tenant.to_string(),
            repo.to_string(),
            branch.to_string(),
            SYSTEM_WORKSPACE.to_string(),
        )
        .with_auth(AuthContext::system())
    }

    /// Best-effort detection of active replication (for the no-locks warning).
    pub(super) fn replication_active(&self) -> bool {
        self.storage.config.replication_enabled
    }
}

/// Resolved pieces needed to invoke a mount's adapter, produced by
/// [`VirtualMountSyncHandler::build_ctx_parts`].
pub(super) struct CtxParts {
    /// Integration node id (used to cache resolved capabilities).
    pub(super) integ_node_id: String,
    pub(super) adapter_path: String,
    pub(super) credential: Option<Value>,
    pub(super) mount_snapshot: Value,
    pub(super) scope: MountScope,
    pub(super) public_origin: Option<String>,
}
