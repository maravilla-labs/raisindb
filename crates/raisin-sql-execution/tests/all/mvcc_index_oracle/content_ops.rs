//! The content write: one node's properties (and optionally type) through a
//! chosen funnel — the transaction layer, the repository layer, or SQL.

use super::driver::Run;
use super::env::{lit, query, MAIN, WS};
use super::ops::Funnel;
use raisin_storage::{NodeRepository, Storage, UpdateNodeOptions};
use std::collections::HashMap;

impl Run {
    /// Replace `id`'s content (and optionally its type) through `funnel`.
    pub async fn write_content(
        &mut self,
        id: &str,
        props: HashMap<String, raisin_models::nodes::properties::PropertyValue>,
        retype: Option<&str>,
        funnel: Funnel,
    ) -> Option<String> {
        let mut props = props;
        Self::refresh_refs(&self.tree, &mut props);
        {
            let n = self.tree.nodes.get_mut(id).unwrap();
            n.props = props.clone();
            if let Some(t) = retype {
                n.node_type = t.to_string();
            }
            n.updated = self.seq;
        }
        let node = Self::wire_node(&self.tree, id);
        let desc = format!("update {id} via {funnel:?} retype={retype:?}");
        let res = match funnel {
            Funnel::Tx => {
                let ctx = self.env.tx(MAIN).await;
                // A client read-modify-writes: start from the stored node.
                let mut current = match ctx.get_node(WS, id).await {
                    Ok(Some(n)) => n,
                    Ok(None) => return self.fail(&desc, "node not found by tx read"),
                    Err(e) => return self.fail(&desc, e),
                };
                current.properties = node.properties.clone();
                current.node_type = node.node_type.clone();
                match ctx.put_node(WS, &current).await {
                    Ok(()) => ctx.commit().await,
                    Err(e) => Err(e),
                }
            }
            Funnel::Repo => {
                let scope = self.env.scope(MAIN);
                match self.env.storage.nodes().get(scope, id, None).await {
                    Ok(Some(mut current)) => {
                        current.properties = node.properties.clone();
                        current.node_type = node.node_type.clone();
                        let options = UpdateNodeOptions {
                            validate_schema: false,
                            allow_type_change: true,
                            operation_meta: None,
                        };
                        self.env
                            .storage
                            .nodes()
                            .update(scope, current, options)
                            .await
                    }
                    Ok(None) => return self.fail(&desc, "node not found by repo read"),
                    Err(e) => Err(e),
                }
            }
            Funnel::Sql => {
                let json = serde_json::to_string(&props).expect("props json");
                let sql = format!(
                    "UPDATE '{WS}' SET properties = '{}'::jsonb WHERE id = '{id}'",
                    lit(&json)
                );
                query(&self.env.engine(MAIN), &sql)
                    .await
                    .map(|_| ())
                    .map_err(raisin_error::Error::Backend)
            }
        };
        match res {
            Ok(()) => Some(desc),
            Err(e) => self.fail(&desc, e),
        }
    }
}
