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

mod ast;
mod executor;
// reserved for future: mod parser;
pub use ast::*;
pub use executor::*;

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models as models;
    use serde_json::Value;
    use std::collections::HashMap;
    fn node(
        id: &str,
        name: &str,
        path: &str,
        t: &str,
        parent: Option<&str>,
    ) -> models::nodes::Node {
        models::nodes::Node {
            id: id.into(),
            name: name.into(),
            path: path.into(),
            node_type: t.into(),
            archetype: None,
            properties: Default::default(),
            children: vec![],
            parent: parent.map(|s| s.into()),
            version: 1,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            tenant_id: None,
            workspace: None,
            owner_id: None,
            order_key: todo!(),
            has_children: todo!(),
            relations: todo!(),
        }
    }

    #[test]
    fn filter_like_and_in() {
        let nodes = vec![
            node("a", "A", "/a", "alpha", None),
            node("b", "B", "/b", "beta", Some("/a")),
        ];
        let q = NodeSearchQuery {
            and: Some(vec![
                Filter::Field(FieldFilter(HashMap::from([(
                    "path".into(),
                    FieldOperators {
                        like: Some("/".into()),
                        ..Default::default()
                    },
                )]))),
                Filter::Field(FieldFilter(HashMap::from([(
                    "nodeType".into(),
                    FieldOperators {
                        in_: Some(vec![Value::String("beta".into())]),
                        ..Default::default()
                    },
                )]))),
            ]),
            order_by: Some(HashMap::from([("path".into(), SortOrder::Asc)])),
            limit: Some(10),
            offset: Some(0),
            ..Default::default()
        };
        let out = eval_query(&nodes, &q);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "b");
    }
}
