//! Push subscriptions outside a sync run: teardown on disable, and the periodic
//! renewal sweep.

use super::*;

impl VirtualMountSyncHandler {
    /// Best-effort push teardown for a disabled mount: unsubscribe the provider
    /// subscription and clear the push fields. Never fails the caller.
    pub(super) async fn teardown_push(
        &self,
        svc: &NodeService<RocksDBStorage>,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        mount: &MountConfig,
    ) {
        if mount.state.push_subscription_id.is_none() {
            return;
        }
        let Some(invoker) = self.invoker.clone() else {
            return;
        };
        let Ok(parts) = self
            .build_ctx_parts(svc, tenant, repo, config_branch, mount)
            .await
        else {
            return;
        };
        let ctx = subscription::adapter_ctx(
            self.storage.clone(),
            parts.scope,
            config_branch.to_string(),
            mount.clone(),
            parts.adapter_path,
            invoker.as_ref(),
            self.materializer.as_ref(),
            parts.credential,
            parts.mount_snapshot,
            parts.public_origin,
        );
        let mut state = mount.state.clone();
        subscription::teardown(&ctx, &mut state).await;
        let _ = persist_mount_state(
            &self.storage,
            tenant,
            repo,
            config_branch,
            &mount.mount_id,
            &mut state,
        )
        .await;
    }

    /// Periodic scan (mirrors `check::run_check`): renew every active push
    /// subscription whose provider expiry is within [`subscription::RENEW_WINDOW_SECS`].
    /// Returns the number of subscriptions renewed.
    pub(super) async fn run_subscription_renew(
        &self,
        tenant_filter: Option<String>,
    ) -> Result<usize> {
        let Some(invoker) = self.invoker.clone() else {
            return Ok(0);
        };
        let now = Utc::now().timestamp();
        let tenants = match &tenant_filter {
            Some(t) => vec![t.clone()],
            None => crate::management::list_tenants(&self.storage).await?,
        };

        let mut renewed = 0usize;
        for tenant in tenants {
            let repos = match crate::management::list_repositories(&self.storage, &tenant).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, error = %e, "vmount-renew: list repos failed");
                    continue;
                }
            };
            for repo in repos {
                let config_branch = self
                    .storage
                    .repository_management()
                    .get_repository(&tenant, &repo)
                    .await
                    .ok()
                    .flatten()
                    .map(|r| r.config.default_branch)
                    .unwrap_or_else(|| "main".to_string());
                let svc = self.system_service(&tenant, &repo, &config_branch);
                let mounts = match svc.list_by_type("raisin:VirtualMount").await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(tenant = %tenant, repo = %repo, error = %e, "vmount-renew: list mounts failed");
                        continue;
                    }
                };
                for node in mounts {
                    let Ok(mount) = MountConfig::from_node(&node) else {
                        continue;
                    };
                    if !mount.enabled
                        || mount.state.push_status.as_deref() != Some("active")
                        || !mount
                            .state
                            .push_expires_within(now, subscription::RENEW_WINDOW_SECS)
                    {
                        continue;
                    }
                    let Ok(parts) = self
                        .build_ctx_parts(&svc, &tenant, &repo, &config_branch, &mount)
                        .await
                    else {
                        continue;
                    };
                    let ctx = subscription::adapter_ctx(
                        self.storage.clone(),
                        parts.scope,
                        config_branch.clone(),
                        mount.clone(),
                        parts.adapter_path,
                        invoker.as_ref(),
                        self.materializer.as_ref(),
                        parts.credential,
                        parts.mount_snapshot,
                        parts.public_origin,
                    );
                    let mut state = mount.state.clone();
                    if subscription::renew(&ctx, &mut state).await {
                        renewed += 1;
                    }
                    let _ = persist_mount_state(
                        &self.storage,
                        &tenant,
                        &repo,
                        &config_branch,
                        &mount.mount_id,
                        &mut state,
                    )
                    .await;
                }
            }
        }
        Ok(renewed)
    }
}
