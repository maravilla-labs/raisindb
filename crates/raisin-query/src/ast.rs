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

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct NodeSearchQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub and: Option<Vec<Filter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<Filter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not: Option<Box<Filter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_by: Option<HashMap<String, SortOrder>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum Filter {
    And(AndFilter),
    Or(OrFilter),
    Not(NotFilter),
    Field(FieldFilter),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AndFilter {
    pub and: Vec<Filter>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OrFilter {
    pub or: Vec<Filter>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NotFilter {
    pub not: Box<Filter>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FieldFilter(pub HashMap<String, FieldOperators>);

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "lowercase")]
pub struct FieldOperators {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eq: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ne: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub like: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "in")]
    pub in_: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exists: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gt: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lt: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gte: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lte: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Asc,
    Desc,
}
