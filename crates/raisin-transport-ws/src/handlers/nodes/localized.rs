// SPDX-License-Identifier: BSL-1.1

//! `node_get_by_localized_path { locale, path }` (plan Phase 12): a node by
//! its localized URL, through the one core lookup
//! (`NodeService::resolve_localized_path`). Not found, forbidden and hidden
//! in the locale all answer `null`.

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{NodeGetByLocalizedPathPayload, RequestEnvelope, ResponseEnvelope},
};

use super::helpers::{build_node_service, extract_context};

pub async fn handle_node_get_by_localized_path<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeGetByLocalizedPathPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);
    let found = node_service
        .resolve_localized_path(&payload.locale, &payload.path)
        .await?;
    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(found)?,
    )))
}

#[cfg(test)]
mod tests {
    use crate::protocol::{NodeGetByLocalizedPathPayload, RequestType};

    #[test]
    fn the_request_type_and_payload_have_their_wire_names() {
        let ty: RequestType = serde_json::from_str("\"node_get_by_localized_path\"").unwrap();
        assert_eq!(ty, RequestType::NodeGetByLocalizedPath);
        let payload: NodeGetByLocalizedPathPayload =
            serde_json::from_value(serde_json::json!({ "locale": "fr", "path": "/produits" }))
                .unwrap();
        assert_eq!(
            (payload.locale.as_str(), payload.path.as_str()),
            ("fr", "/produits")
        );
    }
}
