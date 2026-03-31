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

use crate::nodes::properties::PropertyValue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Container {
        #[serde(skip_serializing_if = "Option::is_none")]
        direction: Option<LayoutDirection>, // Vertical or Horizontal stacking
        #[serde(skip_serializing_if = "Option::is_none")]
        spacing: Option<u32>, // Space between elements
        #[serde(skip_serializing_if = "Option::is_none")]
        alignment: Option<Alignment>, // Alignment of children
        children: Vec<LayoutNode>, // Nested layout nodes
    },
    Group {
        label: String, // Label for the group
        #[serde(skip_serializing_if = "Option::is_none")]
        direction: Option<LayoutDirection>, // Direction of the group's children
        #[serde(skip_serializing_if = "Option::is_none")]
        spacing: Option<u32>, // Space between children
        children: Vec<LayoutNode>, // Nested layout nodes
    },
    TabPanel {
        tabs: Vec<Tab>, // Definition of tabs
    },
    Field {
        name: String, // Name of the field in the schema
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<String>, // Width of the field (e.g., "50%")
        #[serde(skip_serializing_if = "Option::is_none")]
        condition: Option<Box<Condition>>, // Conditional rendering
    },
    Grid {
        rows: u32,                 // Number of rows
        columns: u32,              // Number of columns
        children: Vec<LayoutNode>, // Fields or other layout nodes
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Tab {
    pub name: String, // Name of the tab
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<LayoutDirection>, // Layout direction within the tab
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<u32>, // Space between children
    pub children: Vec<LayoutNode>, // Nested layout nodes in the tab
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct Condition {
    pub field: String,               // Field to evaluate
    pub operator: ConditionOperator, // Operator for comparison
    pub value: PropertyValue,        // Value to compare against
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum ConditionOperator {
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    Contains,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum LayoutDirection {
    Vertical,
    Horizontal,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum Alignment {
    Leading,
    Center,
    Trailing,
}

// REMOVE: impl LayoutNode {
// REMOVE:     /// Generates the JSON schema for the `LayoutNode` struct.
// REMOVE:     pub fn json_schema() -> schemars::schema::RootSchema {
// REMOVE:         schema_for!(LayoutNode)
// REMOVE:     }
// REMOVE: }
