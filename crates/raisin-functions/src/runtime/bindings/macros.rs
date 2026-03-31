// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Macros for defining API bindings
//!
//! The `define_api_method!` macro provides a convenient way to define API method
//! bindings that work with both QuickJS and Starlark runtimes.

/// Define a single API method binding
///
/// # Example
///
/// ```ignore
/// define_api_method!(
///     internal: "nodes_get",
///     js: "get",
///     py: "get",
///     category: "nodes",
///     args: [
///         (workspace, String),
///         (path, String)
///     ],
///     returns: OptionalJson,
///     invoke: |api, args| {
///         let mut parser = ArgParser::new(&args);
///         let workspace = parser.string()?;
///         let path = parser.string()?;
///         let result = api.node_get(&workspace, &path).await?;
///         Ok(InvokeResult::OptionalJson(result))
///     }
/// );
/// ```
#[macro_export]
macro_rules! define_api_method {
    (
        internal: $internal:expr,
        js: $js:expr,
        py: $py:expr,
        category: $category:expr,
        args: [ $( ($arg_name:ident, $arg_type:ident) ),* $(,)? ],
        returns: $return_type:ident,
        invoke: $invoke:expr
    ) => {
        $crate::runtime::bindings::registry::ApiMethodDescriptor {
            internal_name: $internal,
            js_name: $js,
            py_name: $py,
            category: $category,
            args: &[
                $(
                    $crate::runtime::bindings::registry::ArgSpec::new(
                        stringify!($arg_name),
                        $crate::runtime::bindings::registry::ArgType::$arg_type
                    ),
                )*
            ],
            return_type: $crate::runtime::bindings::registry::ReturnType::$return_type,
            invoker: $invoke,
        }
    };
}

/// Helper macro to create an async invoker function
///
/// This macro wraps an async block in a BoxFuture for the invoker signature.
#[macro_export]
macro_rules! api_invoker {
    ($invoke_body:expr) => {
        |api: std::sync::Arc<dyn $crate::api::FunctionApi>,
         args: Vec<serde_json::Value>|
         -> futures::future::BoxFuture<
            'static,
            raisin_error::Result<$crate::runtime::bindings::registry::InvokeResult>,
        > { Box::pin(async move { $invoke_body(api, args).await }) }
    };
}

pub use api_invoker;
pub use define_api_method;
