use std::pin::Pin;
use std::sync::Arc;

use raisin_audit::AuditRepository;
use raisin_models as models;
use raisin_models::nodes::audit_log::AuditLogAction;

use crate::traits::Audit;

pub struct RepoAuditAdapter<A: AuditRepository> {
    pub(crate) inner: Arc<A>,
}

impl<A: AuditRepository> RepoAuditAdapter<A> {
    pub fn new(inner: Arc<A>) -> Self {
        Self { inner }
    }
}

impl<A: AuditRepository> Audit for RepoAuditAdapter<A> {
    fn write<'a>(
        &'a self,
        node: &'a models::nodes::Node,
        action: AuditLogAction,
        details: Option<String>,
    ) -> Pin<Box<dyn std::future::Future<Output = raisin_error::Result<()>> + Send + 'a>> {
        let log = raisin_audit::make_log(
            node.id.clone(),
            node.path.clone(),
            node.workspace.clone().unwrap_or_default(),
            node.updated_by.clone(),
            action,
            details,
        );
        let inner = self.inner.clone();
        Box::pin(async move { inner.write_log(log).await })
    }
}
