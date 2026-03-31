// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Notification API bindings
//!
//! Provides a high-level API for sending system notifications from
//! Starlark/QuickJS functions.
//!
//! Notifications are created directly in the recipient's `/notifications` folder.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::{Error, Result};
use serde_json::{json, Value};
use std::sync::Arc;

/// Get all notification operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // notify(options) - options object with title, body, recipient/recipientId, etc.
        ApiMethodDescriptor {
            internal_name: "notify_send",
            js_name: "notify",
            py_name: "notify",
            category: "notify",
            args: vec![ArgSpec::new("options", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let options = parser.json()?;

                    // Extract fields from options object
                    let title = options
                        .get("title")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            Error::Validation("Missing required field 'title'".to_string())
                        })?
                        .to_string();

                    let body_text = options
                        .get("body")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let recipient = options
                        .get("recipient")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let recipient_id = options
                        .get("recipientId")
                        .or_else(|| options.get("recipient_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let priority = options
                        .get("priority")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as u32)
                        .unwrap_or(3);

                    let notification_type = options
                        .get("type")
                        .or_else(|| options.get("notification_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("system")
                        .to_string();

                    let link = options
                        .get("link")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let data = options.get("data").cloned();

                    // Validate: must have either recipient or recipient_id
                    if recipient.is_none() && recipient_id.is_none() {
                        return Err(Error::Validation(
                            "Must provide either 'recipient' (path) or 'recipientId' (UUID)"
                                .to_string(),
                        ));
                    }

                    // Resolve recipient path
                    // Users are stored in the "raisin:access_control" workspace
                    let workspace = "raisin:access_control";
                    let recipient_path = if let Some(path) = recipient {
                        path
                    } else if let Some(ref id) = recipient_id {
                        // Look up user by ID to get their path
                        let user = api.node_get_by_id(workspace, id).await?.ok_or_else(|| {
                            Error::NotFound(format!("User with ID '{}' not found", id))
                        })?;
                        user.get("path")
                            .and_then(|p| p.as_str())
                            .ok_or_else(|| {
                                Error::Internal("User node missing 'path' field".to_string())
                            })?
                            .to_string()
                    } else {
                        return Err(Error::Validation(
                            "Must provide either 'recipient' (path) or 'recipientId' (UUID)"
                                .to_string(),
                        ));
                    };

                    // Build notification folder path: {recipient_path}/notifications
                    let notifications_folder = format!("{}/notifications", recipient_path);

                    // Verify the notifications folder exists
                    let folder_exists = api.node_get(workspace, &notifications_folder).await?;
                    if folder_exists.is_none() {
                        return Err(Error::NotFound(format!(
                            "Notifications folder not found at '{}'. User may not have been properly initialized.",
                            notifications_folder
                        )));
                    }

                    // Generate a unique slug using current timestamp and random suffix
                    let timestamp = chrono::Utc::now().timestamp_millis();
                    let slug = format!("notif-{}", timestamp);

                    // Build notification properties according to raisin:Notification schema
                    let mut props = json!({
                        "type": notification_type,
                        "title": title,
                        "read": false,
                        "priority": priority
                    });

                    if let Some(ref body) = body_text {
                        props["body"] = json!(body);
                    }
                    if let Some(ref l) = link {
                        props["link"] = json!(l);
                    }
                    if let Some(ref d) = data {
                        props["data"] = d.clone();
                    }

                    // Create the notification node data
                    let notification_data = json!({
                        "slug": slug,
                        "name": slug,
                        "node_type": "raisin:Notification",
                        "properties": props
                    });

                    // Create the notification node directly in user's notifications folder
                    let result = api
                        .node_create(workspace, &notifications_folder, notification_data)
                        .await?;

                    let notification_path = format!("{}/{}", notifications_folder, slug);

                    Ok(InvokeResult::Json(json!({
                        "success": true,
                        "notification_id": result.get("id"),
                        "notification_path": notification_path
                    })))
                })
            },
        },
    ]
}
