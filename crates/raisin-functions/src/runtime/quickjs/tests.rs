// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for the QuickJS runtime.

use super::*;
use crate::api::MockFunctionApi;

#[tokio::test]
async fn test_simple_function() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            return { received: input, processed: true };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({"message": "hello"}));

    let metadata = FunctionMetadata::javascript("test_function");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({
        "tenant_id": "tenant1",
        "repo_id": "repo1",
        "branch": "main"
    })));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
    let output = result.output.unwrap();
    assert_eq!(output["processed"], true);
    assert_eq!(output["received"]["message"], "hello");
}

#[tokio::test]
async fn test_console_logging() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            console.log("Processing input:", JSON.stringify(input));
            console.warn("This is a warning");
            console.error("This is an error");
            return { logged: true };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("log_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
}

#[tokio::test]
async fn test_syntax_error() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input {  // Missing closing paren
            return input;
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("syntax_error_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(!result.success);
    assert!(result.error.is_some());
}

#[tokio::test]
async fn test_missing_entrypoint() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function someOtherFunction(input) {
            return input;
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("missing_handler");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(!result.success);
    assert!(result.error.as_ref().unwrap().message.contains("not found"));
}

#[tokio::test]
async fn test_context_access() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            return {
                tenant: raisin.context.tenant_id,
                branch: raisin.context.branch
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("context_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({
        "tenant_id": "tenant1",
        "repo_id": "repo1",
        "branch": "main"
    })));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
    let output = result.output.unwrap();
    assert_eq!(output["tenant"], "tenant1");
    assert_eq!(output["branch"], "main");
}

#[tokio::test]
async fn test_json_manipulation() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const result = {
                original: input,
                modified: {
                    ...input,
                    extra: "field",
                    count: (input.items || []).length
                }
            };
            return result;
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user").with_input(
        serde_json::json!({
            "name": "test",
            "items": [1, 2, 3]
        }),
    );

    let metadata = FunctionMetadata::javascript("json_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
    let output = result.output.unwrap();
    assert_eq!(output["modified"]["extra"], "field");
    assert_eq!(output["modified"]["count"], 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_nodes_api() {
    let runtime = QuickJsRuntime::new();

    // Test that raisin.nodes.get returns data from the mock API
    let code = r#"
        function handler(input) {
            const node = raisin.nodes.get("default", "/test-path");
            return {
                nodeId: node.id,
                nodePath: node.path,
                nodeType: node.node_type
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("nodes_api_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({
        "tenant_id": "tenant1",
        "repo_id": "repo1",
        "branch": "main"
    })));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
    let output = result.output.unwrap();
    assert_eq!(output["nodeId"], "mock-node-id");
    assert_eq!(output["nodePath"], "/test-path");
    assert_eq!(output["nodeType"], "raisin:Page");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_nodes_update_property() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            // Update a property on a node
            const success = raisin.nodes.updateProperty(
                "default",
                "/test-path",
                "status",
                "completed"
            );

            // Also test with nested property path
            const nestedSuccess = raisin.nodes.updateProperty(
                "default",
                "/test-path",
                "metadata.count",
                42
            );

            // And with an object value
            const objectSuccess = raisin.nodes.updateProperty(
                "default",
                "/test-path",
                "data",
                { key: "value", nested: { field: true } }
            );

            return {
                success: success,
                nestedSuccess: nestedSuccess,
                objectSuccess: objectSuccess
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("nodes_update_property_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({
        "tenant_id": "tenant1",
        "repo_id": "repo1",
        "branch": "main"
    })));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
    let output = result.output.unwrap();
    assert_eq!(output["success"], true);
    assert_eq!(output["nestedSuccess"], true);
    assert_eq!(output["objectSuccess"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_sql_api() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const result = raisin.sql.query("SELECT * FROM nodes WHERE id = $1", ["123"]);
            return {
                columns: result.columns,
                rowCount: result.row_count
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("sql_api_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
    let output = result.output.unwrap();
    assert_eq!(output["columns"], serde_json::json!(["id", "name"]));
    assert_eq!(output["rowCount"], 1);
}

#[test]
fn test_has_es6_modules() {
    use module_loader::has_es6_modules;

    // Positive cases
    assert!(has_es6_modules("import { foo } from './utils.js'"));
    assert!(has_es6_modules("import foo from './utils.js'"));
    assert!(has_es6_modules("import{foo} from './utils.js'"));
    assert!(has_es6_modules("export function bar() {}"));
    assert!(has_es6_modules("export default handler"));
    assert!(has_es6_modules("export { foo, bar }"));
    assert!(has_es6_modules("export{foo}"));

    // Negative cases
    assert!(!has_es6_modules(
        "function handler(input) { return input; }"
    ));
    assert!(!has_es6_modules("const foo = 'import bar';"));
    assert!(!has_es6_modules("// import something"));
}

#[test]
fn test_module_resolver_path_resolution() {
    use module_loader::FunctionModuleResolver;

    let files = Arc::new(HashMap::from([
        ("index.js".to_string(), "".to_string()),
        ("utils.js".to_string(), "".to_string()),
        ("lib/helpers.js".to_string(), "".to_string()),
    ]));

    let resolver = FunctionModuleResolver::new(files);

    // Relative paths from root
    assert_eq!(resolver.resolve_path("index.js", "./utils.js"), "utils.js");
    assert_eq!(
        resolver.resolve_path("index.js", "./lib/helpers.js"),
        "lib/helpers.js"
    );

    // Relative paths from subdirectory
    assert_eq!(
        resolver.resolve_path("lib/helpers.js", "../utils.js"),
        "utils.js"
    );
    assert_eq!(
        resolver.resolve_path("lib/helpers.js", "./sibling.js"),
        "lib/sibling.js"
    );

    // Auto-append .js extension
    assert_eq!(resolver.resolve_path("index.js", "./utils"), "utils.js");
}

/// One shared library importing another — the resolver's half of the contract.
///
/// The key `load_external_modules` writes for a nested external is
/// `<dir>/<path-under-dir>`, and this is the assertion that the resolver
/// produces exactly that string from the specifier. The two agreed all along;
/// what failed was DISCOVERY — `collect_external_import_dirs` ignored
/// `export … from` and never scanned the externals it had loaded, so the file
/// was simply absent from the map and a correct resolution was rejected as
/// unresolvable.
#[test]
fn a_library_can_resolve_another_library() {
    use module_loader::FunctionModuleResolver;

    let files = Arc::new(HashMap::from([
        ("index.js".to_string(), String::new()),
        ("studio-mcp-shared/refs.js".to_string(), String::new()),
        // Keyed as load_external_modules keys it: dir_name + path under it.
        ("refs/ref-shared/refs.js".to_string(), String::new()),
    ]));
    let resolver = FunctionModuleResolver::new(files);

    assert_eq!(
        resolver.resolve_path("studio-mcp-shared/refs.js", "../refs/ref-shared/refs.js"),
        "refs/ref-shared/refs.js",
    );
    // And the direct case a function entry uses, which never broke.
    assert_eq!(
        resolver.resolve_path("index.js", "../refs/ref-shared/refs.js"),
        "refs/ref-shared/refs.js",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_ai_api_completion() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const response = raisin.ai.completion({
                model: "gpt-4o",
                messages: [
                    { role: "system", content: "You are helpful" },
                    { role: "user", content: "Hello!" }
                ],
                temperature: 0.7,
                max_tokens: 1000
            });
            return {
                aiModel: response.model,
                responseContent: response.message.content,
                hasUsage: !!response.usage
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("ai_api_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({
        "tenant_id": "tenant1",
        "repo_id": "repo1",
        "branch": "main"
    })));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
    let output = result.output.unwrap();
    assert_eq!(output["aiModel"], "gpt-4o");
    assert!(output["responseContent"]
        .as_str()
        .unwrap()
        .contains("Hello!"));
    assert_eq!(output["hasUsage"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_ai_api_list_models() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const models = raisin.ai.listModels();
            return {
                modelCount: models.length,
                hasGpt4: models.some(m => m.id === "gpt-4o"),
                hasClaude: models.some(m => m.id === "claude-3-5-sonnet"),
                firstModel: models[0]
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("ai_list_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
    let output = result.output.unwrap();
    assert!(output["modelCount"].as_u64().unwrap() >= 2);
    assert_eq!(output["hasGpt4"], true);
    assert_eq!(output["hasClaude"], true);
    assert_eq!(output["firstModel"]["id"], "gpt-4o");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_ai_api_default_model() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const chatModel = raisin.ai.getDefaultModel("chat");
            const agentModel = raisin.ai.getDefaultModel("agent");
            const unknownModel = raisin.ai.getDefaultModel("unknown");
            return {
                chatModel: chatModel,
                agentModel: agentModel,
                unknownModel: unknownModel || null
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("ai_default_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success);
    let output = result.output.unwrap();
    assert_eq!(output["chatModel"], "gpt-4o");
    assert_eq!(output["agentModel"], "claude-3-5-sonnet");
    assert_eq!(output["unknownModel"], serde_json::Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_functions_execute() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            console.log("Starting function execute test");

            try {
                const result = raisin.functions.execute(
                    "/functions/tools/get-weather",
                    { location: "NYC" },
                    {
                        tool_call_path: "/chats/c1/msg-2/tool-call-1",
                        tool_call_workspace: "content"
                    }
                );
                console.log("Execute result:", JSON.stringify(result));
                return {
                    hasResult: !!result,
                    functionPath: result.function_path,
                    argsLocation: result.arguments.location
                };
            } catch (error) {
                console.error("Error executing function:", error);
                return { error: String(error) };
            }
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("functions_execute_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({
        "tenant_id": "tenant1",
        "repo_id": "repo1",
        "branch": "main"
    })));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    // Print logs for debugging
    for log in &result.logs {
        println!("[{}] {}", log.level, log.message);
    }

    if !result.success {
        if let Some(err) = &result.error {
            println!("Execution error: {:?}", err);
        }
    }

    assert!(result.success, "Function execution failed");
    let output = result.output.unwrap();

    // Check if there was an error in JS
    if let Some(error) = output.get("error") {
        panic!("JavaScript error: {}", error);
    }

    assert_eq!(output["hasResult"], true);
    assert_eq!(output["functionPath"], "/functions/tools/get-weather");
    assert_eq!(output["argsLocation"], "NYC");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_nodes_transaction_api() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            console.log("Starting transaction test");

            try {
                // Begin transaction
                const ctx = raisin.nodes.beginTransaction();
                console.log("Transaction started");

                // Create a node within the transaction
                const msg1 = ctx.create('default', '/chat', {
                    name: 'msg-1',
                    node_type: 'raisin:AIMessage',
                    properties: { role: 'user', content: 'Hello' }
                });
                console.log("Created msg1:", JSON.stringify(msg1));

                // Create another node
                const msg2 = ctx.create('default', '/chat', {
                    name: 'msg-2',
                    node_type: 'raisin:AIMessage',
                    properties: { role: 'assistant', content: 'Hi!' }
                });
                console.log("Created msg2:", JSON.stringify(msg2));

                // Add a node (uses provided path)
                const added = ctx.add('default', {
                    path: '/chat/config',
                    name: 'config',
                    node_type: 'raisin:Config',
                    properties: { theme: 'dark' }
                });
                console.log("Added node:", JSON.stringify(added));

                // Get a node by path within transaction
                const fetched = ctx.getByPath('default', '/chat/msg-1');
                console.log("Fetched by path:", JSON.stringify(fetched));

                // Skip listChildren for now - there's a type coercion issue
                // TODO: debug the tx_list_children internal function

                // Set actor and message
                ctx.setActor('test-user');
                ctx.setMessage('Test commit from JS');

                // Commit
                ctx.commit();
                console.log("Transaction committed");

                return {
                    success: true,
                    msg1Id: msg1.id,
                    msg1Path: msg1.path,
                    msg2Id: msg2.id,
                    addedPath: added.path,
                    fetchedPath: fetched ? fetched.path : null
                };
            } catch (error) {
                console.error("Transaction error:", error.message || error);
                return { success: false, error: String(error) };
            }
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("transaction_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({
        "tenant_id": "tenant1",
        "repo_id": "repo1",
        "branch": "main"
    })));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    // Print logs for debugging
    for log in &result.logs {
        println!("[{}] {}", log.level, log.message);
    }

    if !result.success {
        if let Some(err) = &result.error {
            println!("Execution error: {:?}", err);
        }
    }

    assert!(result.success, "Function execution failed");
    let output = result.output.unwrap();

    // Check if there was an error in JS
    if let Some(error) = output.get("error") {
        panic!("JavaScript error: {}", error);
    }

    assert_eq!(output["success"], true, "Transaction should succeed");
    assert!(!output["msg1Id"].is_null(), "msg1 should have an ID");
    assert_eq!(
        output["msg1Path"], "/chat/msg-1",
        "msg1 path should be correct"
    );
    assert!(!output["msg2Id"].is_null(), "msg2 should have an ID");
    assert_eq!(
        output["addedPath"], "/chat/config",
        "added node path should be correct"
    );
    assert!(
        output["fetchedPath"].is_string(),
        "fetched node should have a path"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_nodes_transaction_rollback() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            console.log("Starting rollback test");

            try {
                const ctx = raisin.nodes.beginTransaction();

                // Create a node
                ctx.create('default', '/chat', {
                    name: 'should-not-exist',
                    node_type: 'raisin:Message',
                    properties: {}
                });

                // Explicitly rollback
                ctx.rollback();
                console.log("Transaction rolled back");

                return { success: true, rolledBack: true };
            } catch (error) {
                console.error("Error:", error);
                return { success: false, error: String(error) };
            }
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("rollback_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "Function execution failed");
    let output = result.output.unwrap();
    assert_eq!(output["success"], true);
    assert_eq!(output["rolledBack"], true);
}

/// Regression: `createDeep`/`upsertDeep` must be available directly on
/// `raisin.nodes`, not only on a transaction object. The reported bug was
/// `raisin.nodes.createDeep is not a function` because the method only existed
/// inside `beginTransaction()`. The top-level helper wraps its own transaction.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_nodes_create_deep_top_level() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            if (typeof raisin.nodes.createDeep !== 'function') {
                return { success: false, error: 'createDeep is not a function' };
            }
            if (typeof raisin.nodes.upsertDeep !== 'function') {
                return { success: false, error: 'upsertDeep is not a function' };
            }
            const created = raisin.nodes.createDeep('default', '/reports/2026', {
                name: 'q2',
                node_type: 'raisin:Folder',
                properties: { title: 'Q2' }
            });
            raisin.nodes.upsertDeep('default', {
                path: '/reports/2026/q2',
                name: 'q2',
                node_type: 'raisin:Folder',
                properties: { title: 'Q2 updated' }
            });
            return { success: true, createdName: created.name };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("create_deep_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({
        "tenant_id": "tenant1",
        "repo_id": "repo1",
        "branch": "main"
    })));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    for log in &result.logs {
        println!("[{}] {}", log.level, log.message);
    }

    assert!(result.success, "Function execution failed");
    let output = result.output.unwrap();
    assert_eq!(
        output["success"],
        true,
        "createDeep/upsertDeep should succeed: {:?}",
        output.get("error")
    );
    assert_eq!(output["createdName"], "q2");
}

// ============= Timeout / interrupt backstop =============

/// A busy loop never yields to the event loop, so `tokio::time::timeout`
/// alone can never preempt it — without the engine-level interrupt handler
/// this test would spin a worker thread forever (the production symptom was
/// an AI tool call stuck at status "running" while a worker thread burned
/// 100% CPU until process restart). The interrupt handler must convert it
/// into a graceful timeout failure within the configured budget.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_busy_loop_is_interrupted_at_deadline() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            let x = 0;
            while (true) { x = (x + 1) % 1000000; }
            return { x };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("busy_loop_test")
        .with_resource_limits(crate::types::ResourceLimits::default().with_timeout_ms(500));
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let started = Instant::now();
    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert!(!result.success, "busy loop must not succeed");
    let err = result.error.expect("busy loop must produce an error");
    assert!(
        err.message.to_lowercase().contains("timed out")
            || err.message.to_lowercase().contains("timeout"),
        "error must be reported as a timeout, got: {}",
        err.message
    );
    // Generous bound: deadline is 500ms; the interrupt handler fires within
    // the interpreter's polling cadence. Anything below 10s proves the loop
    // was preempted rather than running unbounded.
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "busy loop must be interrupted near the deadline, took {:?}",
        elapsed
    );
}

/// A later execution on the same runtime must not be killed by the previous
/// execution's (already expired) interrupt deadline.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_interrupt_deadline_does_not_leak_to_next_execution() {
    let runtime = QuickJsRuntime::new();

    let spin = r#"
        function handler(input) { while (true) {} }
    "#;
    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("spin")
        .with_resource_limits(crate::types::ResourceLimits::default().with_timeout_ms(300));
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
    let result = runtime
        .execute(spin, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();
    assert!(!result.success);

    // Second execution with a normal budget must succeed.
    let ok_code = r#"
        function handler(input) { return { ok: true }; }
    "#;
    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("ok_fn");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
    let result = runtime
        .execute(ok_code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();
    assert!(
        result.success,
        "execution after an interrupted one must succeed: {:?}",
        result.error
    );
    assert_eq!(result.output.unwrap()["ok"], true);
}

// ============= W3C Fetch API Tests =============

#[tokio::test]
async fn test_fetch_globals_exist() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            return {
                hasFetch: typeof fetch === 'function',
                hasRequest: typeof Request === 'function',
                hasResponse: typeof Response === 'function',
                hasHeaders: typeof Headers === 'function',
                hasAbortController: typeof AbortController === 'function',
                hasAbortSignal: typeof AbortSignal === 'function',
                hasFormData: typeof FormData === 'function',
                hasReadableStream: typeof ReadableStream === 'function',
                hasDOMException: typeof DOMException === 'function'
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("fetch_globals_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(
        result.success,
        "Function execution failed: {:?}",
        result.error
    );
    let output = result.output.unwrap();
    assert_eq!(output["hasFetch"], true, "fetch should be defined");
    assert_eq!(output["hasRequest"], true, "Request should be defined");
    assert_eq!(output["hasResponse"], true, "Response should be defined");
    assert_eq!(output["hasHeaders"], true, "Headers should be defined");
    assert_eq!(
        output["hasAbortController"], true,
        "AbortController should be defined"
    );
    assert_eq!(
        output["hasAbortSignal"], true,
        "AbortSignal should be defined"
    );
    assert_eq!(output["hasFormData"], true, "FormData should be defined");
    assert_eq!(
        output["hasReadableStream"], true,
        "ReadableStream should be defined"
    );
    assert_eq!(
        output["hasDOMException"], true,
        "DOMException should be defined"
    );
}

#[tokio::test]
async fn test_headers_class() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const headers = new Headers();
            headers.set('Content-Type', 'application/json');
            headers.append('Accept', 'text/html');
            headers.append('Accept', 'application/json');

            return {
                contentType: headers.get('content-type'),
                accept: headers.get('Accept'),
                hasContentType: headers.has('Content-Type'),
                caseInsensitive: headers.get('CONTENT-TYPE') === headers.get('content-type')
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("headers_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(
        result.success,
        "Function execution failed: {:?}",
        result.error
    );
    let output = result.output.unwrap();
    assert_eq!(output["contentType"], "application/json");
    assert_eq!(output["accept"], "text/html, application/json");
    assert_eq!(output["hasContentType"], true);
    assert_eq!(output["caseInsensitive"], true);
}

#[tokio::test]
async fn test_abort_controller() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const controller = new AbortController();
            const signal = controller.signal;

            const beforeAbort = signal.aborted;
            controller.abort('User cancelled');
            const afterAbort = signal.aborted;

            return {
                beforeAbort,
                afterAbort,
                signalLinked: controller.signal === signal
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("abort_controller_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(
        result.success,
        "Function execution failed: {:?}",
        result.error
    );
    let output = result.output.unwrap();
    assert_eq!(output["beforeAbort"], false);
    assert_eq!(output["afterAbort"], true);
    assert_eq!(output["signalLinked"], true);
}

#[tokio::test]
async fn test_request_class() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const request = new Request('https://api.example.com/data', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ key: 'value' })
            });

            return {
                url: request.url,
                method: request.method,
                hasHeaders: request.headers instanceof Headers,
                contentType: request.headers.get('content-type')
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("request_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(
        result.success,
        "Function execution failed: {:?}",
        result.error
    );
    let output = result.output.unwrap();
    assert_eq!(output["url"], "https://api.example.com/data");
    assert_eq!(output["method"], "POST");
    assert_eq!(output["hasHeaders"], true);
    assert_eq!(output["contentType"], "application/json");
}

#[tokio::test]
async fn test_response_static_methods() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const jsonResponse = Response.json({ message: 'hello' });
            const redirectResponse = Response.redirect('https://example.com', 302);

            return {
                jsonStatus: jsonResponse.status,
                jsonContentType: jsonResponse.headers.get('content-type'),
                redirectStatus: redirectResponse.status,
                redirectLocation: redirectResponse.headers.get('location')
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("response_static_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(
        result.success,
        "Function execution failed: {:?}",
        result.error
    );
    let output = result.output.unwrap();
    assert_eq!(output["jsonStatus"], 200);
    assert_eq!(output["jsonContentType"], "application/json");
    assert_eq!(output["redirectStatus"], 302);
    assert_eq!(output["redirectLocation"], "https://example.com");
}

#[tokio::test]
async fn test_formdata_class() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const formData = new FormData();
            formData.append('name', 'John');
            formData.append('tags', 'rust');
            formData.append('tags', 'javascript');

            return {
                name: formData.get('name'),
                hasName: formData.has('name'),
                tags: formData.getAll('tags'),
                missingValue: formData.get('missing')
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("formdata_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(
        result.success,
        "Function execution failed: {:?}",
        result.error
    );
    let output = result.output.unwrap();
    assert_eq!(output["name"], "John");
    assert_eq!(output["hasName"], true);
    assert_eq!(output["tags"], serde_json::json!(["rust", "javascript"]));
    assert_eq!(output["missingValue"], serde_json::Value::Null);
}

/// Regression: `raisin.locks.*` and `raisin.inventory.*` must be wired into the
/// QuickJS `__raisin_internal` namespace (this was missing — only the shared
/// registry / Starlark path had them, so JS threw "not a function").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_locks_and_inventory_bindings() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            // Inventory: claim 1 of 2 twice, then sell out.
            const first = raisin.inventory.claim('flight:1', 1, 2);
            const second = raisin.inventory.claim('flight:1', 1, 2);
            const soldOut = raisin.inventory.claim('flight:1', 1, 2);

            // Locks: acquire returns a fence token; second acquire loses.
            const lock = raisin.locks.acquire('seat:14A', 5000);
            const contended = raisin.locks.acquire('seat:14A', 5000);
            const released = raisin.locks.release('seat:14A', lock.token);

            return {
                firstClaimed: first.claimed,
                firstRemaining: first.remaining,
                secondRemaining: second.remaining,
                soldOut: soldOut.claimed,
                acquired: lock.acquired,
                acquiredToken: lock.token,
                contended: contended.acquired,
                released: released,
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("test_locks");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["firstClaimed"], true);
    assert_eq!(output["firstRemaining"], 1);
    assert_eq!(output["secondRemaining"], 0);
    assert_eq!(output["soldOut"], false); // sold out → claimed:false
    assert_eq!(output["acquired"], true);
    assert!(output["acquiredToken"].as_i64().unwrap() > 0);
    assert_eq!(output["contended"], false); // contended → acquired:false
    assert_eq!(output["released"], true);
}

/// `raisin.secrets.*` must be reachable from JavaScript and round-trip through
/// the shared registry. The MockFunctionApi enforces no policy — the policy gate
/// is `RaisinFunctionApi`'s and is tested in `api/raisindb/tests.rs`; what this
/// pins is the JS surface (names, argument order, return shapes).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_secrets_bindings() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const first = raisin.secrets.put('stripe/live', 'sk_v1');
            const readBack = raisin.secrets.get('stripe/live');
            const rotated = raisin.secrets.rotate('stripe/live', 'sk_v2');
            const afterRotate = raisin.secrets.get('stripe/live');
            // A pinned version still resolves to the OLD value.
            const pinned = raisin.secrets.get('stripe/live', 1);

            raisin.secrets.put('sendgrid', 'sg_v1');
            const listed = raisin.secrets.list();

            const tomb = raisin.secrets.delete('sendgrid');

            let deletedThrew = false;
            try { raisin.secrets.get('sendgrid'); } catch (e) { deletedThrew = true; }

            let missingThrew = false;
            try { raisin.secrets.get('never-written'); } catch (e) { missingThrew = true; }

            return {
                firstVersion: first.version,
                firstName: first.name,
                readBack: readBack,
                rotatedVersion: rotated.version,
                afterRotate: afterRotate,
                pinned: pinned,
                listedNames: listed.map(s => s.name),
                listedHasNoValue: listed.every(s => s.value === undefined),
                tombVersion: tomb.version,
                deletedThrew: deletedThrew,
                missingThrew: missingThrew,
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("test_secrets");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["firstVersion"], 1);
    assert_eq!(output["firstName"], "stripe/live");
    assert_eq!(output["readBack"], "sk_v1");
    assert_eq!(output["rotatedVersion"], 2);
    assert_eq!(output["afterRotate"], "sk_v2");
    // Rotation appends: the pinned reference keeps resolving to v1.
    assert_eq!(output["pinned"], "sk_v1");
    assert_eq!(
        output["listedNames"],
        serde_json::json!(["sendgrid", "stripe/live"])
    );
    assert_eq!(output["listedHasNoValue"], true);
    assert_eq!(output["tombVersion"], 2);
    // A deleted secret and a never-written one both THROW — a silent null here
    // would let a function authenticate with nothing.
    assert_eq!(output["deletedThrew"], true);
    assert_eq!(output["missingThrew"], true);
}

/// THE primary use case, end to end in JS: a node property holds a full
/// `secret://name@version` reference, and the function passes that exact string
/// to `get` without stripping anything. Also pins `resolve`'s literal
/// passthrough and the `@`-in-a-name rule.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_secrets_accepts_references() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            // Two versions, so a pin has something to distinguish.
            raisin.secrets.put('node/01H8XY/api_key', 'v1');
            raisin.secrets.rotate('node/01H8XY/api_key', 'v2');

            // What a node property in an `encrypted: true` field actually holds:
            const propertyValue = 'secret://node/01H8XY/api_key@1';

            return {
                // Passed through verbatim — no stripping by the author.
                pinned: raisin.secrets.get(propertyValue),
                unpinned: raisin.secrets.get('secret://node/01H8XY/api_key'),
                bareName: raisin.secrets.get('node/01H8XY/api_key'),
                // resolve() handles both a reference and a plain literal.
                resolvedRef: raisin.secrets.resolve(propertyValue),
                resolvedLiteral: raisin.secrets.resolve('a-plain-password'),
                // A name containing '@' is a NAME, not a pin.
                atName: (function () {
                    raisin.secrets.put('ops@example.com', 'mail-pw');
                    return raisin.secrets.get('secret://ops@example.com');
                })(),
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("test_secret_refs");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    // The pin is honoured: an older node revision resolves to what it held.
    assert_eq!(output["pinned"], "v1");
    assert_eq!(output["unpinned"], "v2");
    assert_eq!(output["bareName"], "v2");
    assert_eq!(output["resolvedRef"], "v1");
    // A non-reference comes back untouched.
    assert_eq!(output["resolvedLiteral"], "a-plain-password");
    assert_eq!(output["atName"], "mail-pw");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_integrations_sync_now_wrapper_round_trips() {
    let runtime = QuickJsRuntime::new();

    // Exercises the raisin.integrations.sync_now wrapper end-to-end: the JS
    // ergonomic wrapper -> __raisin_internal.integrations_sync_now -> FunctionApi
    // -> JSON back into JS. The mock reports { job_id: null, status: "queued" }.
    let code = r#"
        function handler(input) {
            var snake = raisin.integrations.sync_now("mount-abc");
            var camel = raisin.integrations.syncNow("mount-abc", "full");
            return {
                status: snake.status,
                jobIdNull: snake.job_id === null,
                hasStatusKey: ("status" in snake),
                camelStatus: camel.status,
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("test_integrations");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["status"], "queued");
    assert_eq!(output["jobIdNull"], true);
    assert_eq!(output["hasStatusKey"], true);
    assert_eq!(output["camelStatus"], "queued");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_imap_wrapper_round_trips() {
    let runtime = QuickJsRuntime::new();

    // Exercises the raisin.imap.* wrappers end-to-end against the mock backend:
    // JS wrapper -> JSON.stringify(conn) -> __raisin_internal.imap_* -> mock
    // FunctionApi -> JSON back into JS. The mock returns two fixed messages
    // (uids 101, 102) plus a stable mailbox list and message body.
    let code = r#"
        function handler(input) {
            var conn = { host: "imap.example.org", port: 993, tls: true,
                         username: "u@example.org", password: "app-pw" };
            var since = raisin.imap.fetchSince(conn, 100, { mailbox: "INBOX", limit: 50 });
            var boxes = raisin.imap.listMailboxes(conn);
            var msg = raisin.imap.fetchMessage(conn, 102);
            return {
                count: since.messages.length,
                highestUid: since.highestUid,
                uidvalidity: since.uidvalidity,
                firstUid: since.messages[0].uid,
                firstFrom: since.messages[0].from,
                firstSubject: since.messages[0].subject,
                mailboxCount: boxes.length,
                firstMailbox: boxes[0].name,
                msgSubject: msg.subject,
                msgText: msg.text,
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("test_imap");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["count"], 2);
    assert_eq!(output["highestUid"], 102);
    assert_eq!(output["uidvalidity"], 1);
    assert_eq!(output["firstUid"], 101);
    assert_eq!(output["firstFrom"], "alice@example.org");
    assert_eq!(output["firstSubject"], "Mock One");
    assert_eq!(output["mailboxCount"], 2);
    assert_eq!(output["firstMailbox"], "INBOX");
    assert_eq!(output["msgSubject"], "Mock message 102");
    assert_eq!(output["msgText"], "Body of message 102.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_crypto_verify_jwt_roundtrip() {
    let runtime = QuickJsRuntime::new();

    // Exercises raisin.crypto.verifyJwt end-to-end against the mock backend:
    // JS wrapper -> __raisin_internal.crypto_verify_jwt -> mock FunctionApi
    // (deterministic { valid:true, claims:{} }) -> JSON back into JS.
    let code = r#"
        function handler(input) {
            var r = raisin.crypto.verifyJwt("header.payload.sig", {
                jwks_url: "https://issuer.example.org/jwks",
                issuer: "https://issuer.example.org",
                audience: "my-service",
                algorithms: ["RS256"],
            });
            return { valid: r.valid, hasClaims: typeof r.claims === "object" };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("test_crypto_verify_jwt");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["valid"], true);
    assert_eq!(output["hasClaims"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_crypto_sign_and_key_bindings_roundtrip_in_js() {
    let runtime = QuickJsRuntime::new();

    // The full JS path for the four new crypto bindings. The mock delegates
    // these to the REAL `runtime::crypto` primitives (they are pure and
    // offline), so this asserts the actual contract, not a stub: a 64-byte
    // JOSE r||s signature, unpadded base64url, and a known-answer digest.
    let code = r#"
        function handler(input) {
            var kp = raisin.crypto.generateKeyPair();
            var token = raisin.crypto.signJwt(
                { sub: "ticket-1" },
                kp.privateJwk,
                { expiresInSec: 600 }
            );
            var parts = token.split(".");
            return {
                alg: kp.alg,
                hasPrivateD: typeof kp.privateJwk.d === "string",
                publicHasNoD: kp.publicJwk.d === undefined,
                parts: parts.length,
                unpadded: token.indexOf("=") === -1,
                headerAlg: JSON.parse(atob(parts[0].replace(/-/g, "+").replace(/_/g, "/"))).alg,
                sha256abc: raisin.crypto.hash("abc"),
                rand16: raisin.crypto.randomBytes(16),
                randStable: raisin.crypto.randomBytes(16) === raisin.crypto.randomBytes(16)
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("test_crypto_sign_jwt");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["alg"], "ES256");
    assert_eq!(output["hasPrivateD"], true);
    assert_eq!(output["publicHasNoD"], true);
    assert_eq!(output["parts"], 3);
    assert_eq!(output["unpadded"], true);
    assert_eq!(output["headerAlg"], "ES256");
    assert_eq!(
        output["sha256abc"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // randomBytes is the ONE binding the mock stubs (a fixed 'A' pattern), so
    // here we can only assert the length contract; that the real generator
    // produces distinct draws is asserted natively in `runtime::crypto`.
    assert_eq!(
        output["randStable"], true,
        "mock randomBytes is deterministic"
    );
    // base64URL of 16 bytes is 22 chars — unpadded, and with no `+` or `/`.
    // The output goes into URLs, QR payloads and JWS segments, so a `=` here
    // would have to be escaped by every caller.
    let rand16 = output["rand16"].as_str().unwrap();
    assert_eq!(rand16.len(), 22);
    assert!(
        !rand16.contains('=') && !rand16.contains('+') && !rand16.contains('/'),
        "randomBytes must be base64url-unpadded, got {rand16}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_branches_bindings_registered() {
    let runtime = QuickJsRuntime::new();

    // The mock API uses the default trait implementations, which report the
    // capability as unavailable — so the wrapper must (a) expose the
    // raisin.branches namespace with diff/compare functions, and (b) surface
    // the backend error as a thrown Error rather than returning it.
    let code = r#"
        function handler(input) {
            const hasDiff = typeof raisin.branches.diff === "function";
            const hasCompare = typeof raisin.branches.compare === "function";
            const hasCopyNodes = typeof raisin.branches.copyNodes === "function";

            let diffError = null;
            try {
                raisin.branches.diff("feature", "main");
            } catch (e) {
                diffError = String(e.message || e);
            }

            let compareError = null;
            try {
                raisin.branches.compare("feature", "main");
            } catch (e) {
                compareError = String(e.message || e);
            }

            let copyError = null;
            try {
                raisin.branches.copyNodes("feature", "main", { workspace: "default", roots: ["/page"] });
            } catch (e) {
                copyError = String(e.message || e);
            }

            return { hasDiff, hasCompare, hasCopyNodes, diffError, compareError, copyError };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("branches_binding_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["hasDiff"], true);
    assert_eq!(output["hasCompare"], true);
    assert_eq!(output["hasCopyNodes"], true);
    assert!(
        output["diffError"]
            .as_str()
            .unwrap()
            .contains("not available"),
        "expected the default-trait error to be thrown, got {:?}",
        output["diffError"]
    );
    assert!(
        output["copyError"]
            .as_str()
            .unwrap()
            .contains("not available"),
        "expected the default-trait error to be thrown, got {:?}",
        output["copyError"]
    );
    assert!(
        output["compareError"]
            .as_str()
            .unwrap()
            .contains("not available"),
        "expected the default-trait error to be thrown, got {:?}",
        output["compareError"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_scheduler_bindings_registered() {
    let runtime = QuickJsRuntime::new();

    // The mock API uses the default trait implementations, which report the
    // capability as unavailable — so the wrapper must (a) expose the
    // raisin.scheduler namespace with schedule/cancel/list/get functions,
    // and (b) surface the backend error as a thrown Error.
    let code = r#"
        function handler(input) {
            const hasSchedule = typeof raisin.scheduler.schedule === "function";
            const hasCancel = typeof raisin.scheduler.cancel === "function";
            const hasList = typeof raisin.scheduler.list === "function";
            const hasGet = typeof raisin.scheduler.get === "function";

            let scheduleError = null;
            try {
                raisin.scheduler.schedule({
                    targetKind: "function",
                    targetPath: "/lib/hello",
                    runAt: "2030-01-01T00:00:00Z",
                });
            } catch (e) {
                scheduleError = String(e.message || e);
            }

            let cancelError = null;
            try {
                raisin.scheduler.cancel("some-job-id");
            } catch (e) {
                cancelError = String(e.message || e);
            }

            let listError = null;
            try {
                raisin.scheduler.list();
            } catch (e) {
                listError = String(e.message || e);
            }

            let getError = null;
            try {
                raisin.scheduler.get("some-job-id");
            } catch (e) {
                getError = String(e.message || e);
            }

            return { hasSchedule, hasCancel, hasList, hasGet, scheduleError, cancelError, listError, getError };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("scheduler_binding_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["hasSchedule"], true);
    assert_eq!(output["hasCancel"], true);
    assert_eq!(output["hasList"], true);
    assert_eq!(output["hasGet"], true);
    for key in ["scheduleError", "cancelError", "listError", "getError"] {
        assert!(
            output[key].as_str().unwrap().contains("not available"),
            "expected the default-trait error to be thrown for {}, got {:?}",
            key,
            output[key]
        );
    }
}

/// A rejected node WRITE (permission denied, workspace allowlist, path
/// conflict, ...) must surface in JS as a thrown exception — never as a
/// success-shaped return value. This regression-tests the wrapper paths
/// that used to swallow storage errors (create returned an {error} object
/// wrapped as a node; delete/updateProperty returned a bare `false`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_node_write_errors_throw_in_js() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const outcomes = {};

            try {
                const node = raisin.nodes.create("ws", "/parent", { name: "child", node_type: "raisin:Folder" });
                outcomes.create = { threw: false, shape: node && typeof node === 'object' ? Object.keys(node) : node };
            } catch (e) {
                outcomes.create = { threw: true, message: String(e.message || e) };
            }

            try {
                raisin.nodes.update("ws", "/parent/child", { properties: { title: "x" } });
                outcomes.update = { threw: false };
            } catch (e) {
                outcomes.update = { threw: true, message: String(e.message || e) };
            }

            try {
                raisin.nodes.delete("ws", "/parent/child");
                outcomes.delete = { threw: false };
            } catch (e) {
                outcomes.delete = { threw: true, message: String(e.message || e) };
            }

            try {
                raisin.nodes.updateProperty("ws", "/parent/child", "title", "x");
                outcomes.updateProperty = { threw: false };
            } catch (e) {
                outcomes.updateProperty = { threw: true, message: String(e.message || e) };
            }

            try {
                raisin.nodes.move("ws", "/parent/child", "/other");
                outcomes.move = { threw: false };
            } catch (e) {
                outcomes.move = { threw: true, message: String(e.message || e) };
            }

            return outcomes;
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("node_write_error_test");
    let api = Arc::new(
        MockFunctionApi::new(serde_json::json!({}))
            .with_node_write_error("simulated rejection: not allowed"),
    );

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();

    for op in ["create", "update", "delete", "updateProperty", "move"] {
        assert_eq!(
            output[op]["threw"], true,
            "raisin.nodes.{} must throw on a storage error, got {:?}",
            op, output[op]
        );
        assert!(
            output[op]["message"]
                .as_str()
                .unwrap()
                .contains("simulated rejection"),
            "raisin.nodes.{} must carry the storage error message, got {:?}",
            op,
            output[op]["message"]
        );
    }
}

/// An UNCAUGHT storage error from a node write must fail the whole function
/// execution (success = false) instead of silently continuing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_uncaught_node_create_error_fails_function() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const node = raisin.nodes.create("ws", "/parent", { name: "child" });
            return { created: node.path };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("uncaught_node_create_error_test");
    let api = Arc::new(
        MockFunctionApi::new(serde_json::json!({}))
            .with_node_write_error("simulated rejection: not allowed"),
    );

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(
        !result.success,
        "an uncaught node create error must fail the function, got output {:?}",
        result.output
    );
    let err = result.error.expect("error must be reported");
    assert!(
        err.message.contains("simulated rejection"),
        "function error must carry the storage error message, got: {}",
        err.message
    );
}

/// Successful writes keep their success shapes after the error-path rework:
/// create returns the node, delete/updateProperty return true.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_node_write_success_shapes_unchanged() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const node = raisin.nodes.create("ws", "/parent", { name: "child", node_type: "raisin:Folder" });
            const deleted = raisin.nodes.delete("ws", "/parent/child");
            const propSet = raisin.nodes.updateProperty("ws", "/parent/child", "title", "x");
            return { path: node.path, deleted, propSet };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));

    let metadata = FunctionMetadata::javascript("node_write_success_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["path"], "/parent/child");
    assert_eq!(output["deleted"], true);
    assert_eq!(output["propSet"], true);
}

/// A `Resource` passed as an attachment becomes a NODE REFERENCE, not base64.
///
/// This is the difference between the server reading the file under row-level
/// security and the bytes making a pointless round trip through the JS heap —
/// and it has to happen synchronously, because `raisin.email.send` is a sync
/// call and making it async would break a frozen public surface.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resource_attachment_is_coerced_to_a_node_reference() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const stored = new Resource(
                {
                    uuid: 'u1',
                    name: 'ticket.pdf',
                    size: 8,
                    mime_type: 'application/pdf',
                    metadata: { storage_key: 'tenant/2026/ticket.pdf' },
                },
                { workspace: 'assets', nodePath: '/tickets/4711', propertyPath: 'file' }
            );

            const out = __coerceAttachments({
                to: 'user@example.com',
                subject: 'Your ticket',
                text: 'attached',
                attachments: [stored, { content: 'aGk=', filename: 'note.txt' }],
            });

            return {
                coerced: out.attachments[0],
                plainSpecUntouched: out.attachments[1],
                // The caller's own object must not be rewritten under them.
                callerKeptTheResource: true,
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("attachment_coercion_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();
    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();

    let coerced = &output["coerced"];
    assert_eq!(coerced["node"], "/tickets/4711");
    assert_eq!(coerced["workspace"], "assets");
    assert_eq!(coerced["property"], "file");
    assert_eq!(coerced["filename"], "ticket.pdf");
    assert_eq!(coerced["content_type"], "application/pdf");
    // The whole point: no bytes crossed the boundary.
    assert!(
        coerced.get("content").is_none(),
        "a stored resource must not be inlined as base64: {coerced}"
    );

    // A plain spec is passed through untouched.
    assert_eq!(output["plainSpecUntouched"]["content"], "aGk=");
    assert_eq!(output["plainSpecUntouched"]["filename"], "note.txt");
}

/// Surface snapshot: the full raisin.* method inventory, per namespace.
///
/// The public JS surface is a FROZEN contract (the application ecosystem
/// depends on it). This test enumerates `Object.keys` of every namespace —
/// including the transaction object and the `asAdmin()` escalation object —
/// and compares against the expected lists, so any accidental method loss,
/// rename, or unreviewed addition fails loudly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_raisin_api_surface_snapshot() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const names = (o) => Object.keys(o).sort();
            const tx = raisin.nodes.beginTransaction();
            const txKeys = names(tx);
            tx.rollback();
            const admin = raisin.asAdmin();
            return {
                root: names(raisin),
                nodes: names(raisin.nodes),
                sql: names(raisin.sql),
                http: names(raisin.http),
                events: names(raisin.events),
                ai: names(raisin.ai),
                functions: names(raisin.functions),
                flows: names(raisin.flows),
                branches: names(raisin.branches),
                scheduler: names(raisin.scheduler),
                platform: names(raisin.platform),
                tasks: names(raisin.tasks),
                crypto: names(raisin.crypto),
                locks: names(raisin.locks),
                inventory: names(raisin.inventory),
                integrations: names(raisin.integrations),
                imap: names(raisin.imap),
                email: names(raisin.email),
                identities: names(raisin.identities),
                pdf: names(raisin.pdf),
                tx: txKeys,
                admin: names(admin),
                adminNodes: names(admin.nodes),
                adminSql: names(admin.sql),
                hasResourceClass: typeof globalThis.Resource === 'function',
                contextIsObject: typeof raisin.context === 'object',
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("surface_snapshot_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({
        "tenant_id": "tenant1",
        "repo_id": "repo1",
        "branch": "main"
    })));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();

    let expect = |key: &str, mut names: Vec<&str>| {
        names.sort_unstable();
        let expected = serde_json::json!(names);
        assert_eq!(
            output[key], expected,
            "raisin API surface changed for '{}'.\n  expected: {}\n  actual:   {}",
            key, expected, output[key]
        );
    };

    expect(
        "root",
        vec![
            "ai",
            "asAdmin",
            "branches",
            "context",
            "crypto",
            "email",
            "events",
            "flows",
            "functions",
            "http",
            "identities",
            "imap",
            "integrations",
            "inventory",
            "locks",
            "nodes",
            "notify",
            "pdf",
            "platform",
            "scheduler",
            "secrets",
            "sql",
            "tasks",
        ],
    );
    expect(
        "nodes",
        vec![
            "applyChildOrder",
            "beginTransaction",
            "create",
            "createDeep",
            "delete",
            "get",
            "getById",
            "getChildren",
            "history",
            "move",
            // Editorial ordering: name a position or a neighbour; the ordering
            // index mints the fractional key.
            "moveChildAfter",
            "moveChildBefore",
            "query",
            "reorderChild",
            "update",
            "updateProperty",
            "upsertDeep",
        ],
    );
    expect("sql", vec!["execute", "query"]);
    expect("http", vec!["fetch"]);
    expect("events", vec!["emit"]);
    expect(
        "ai",
        vec!["completion", "embed", "getDefaultModel", "listModels"],
    );
    expect("functions", vec!["call", "execute"]);
    expect("flows", vec!["run"]);
    expect("branches", vec!["compare", "copyNodes", "diff"]);
    expect("scheduler", vec!["cancel", "get", "list", "schedule"]);
    expect("platform", vec!["hook"]);
    expect("tasks", vec!["complete", "create"]);
    expect(
        "crypto",
        vec![
            "generateKeyPair",
            "hash",
            "randomBytes",
            "signJwt",
            "uuid",
            "verifyJwt",
        ],
    );
    expect("locks", vec!["acquire", "release", "renew"]);
    expect("inventory", vec!["claim", "release"]);
    expect("integrations", vec!["syncNow", "sync_now"]);
    expect("imap", vec!["fetchMessage", "fetchSince", "listMailboxes"]);
    // `send` and `providers`. A caller may choose WHICH configured sender to
    // use, but not who it is: the identity lives in /config/email, so there is
    // deliberately no per-call "from" surface to widen this to.
    expect("email", vec!["providers", "send"]);
    expect("identities", vec!["findByEmail", "update"]);
    expect(
        "pdf",
        vec!["extractText", "getPageCount", "ocr", "processFromStorage"],
    );
    // tx must NOT expose `move` (intentionally unsupported in transactions).
    expect(
        "tx",
        vec![
            "add",
            "commit",
            "create",
            "createDeep",
            "delete",
            "deleteById",
            "get",
            "getByPath",
            "listChildren",
            "put",
            "rollback",
            "setActor",
            "setMessage",
            "update",
            "updateProperty",
            "upsert",
            "upsertDeep",
        ],
    );
    expect(
        "admin",
        vec![
            "ai",
            "context",
            "events",
            "functions",
            "http",
            "inventory",
            "locks",
            "nodes",
            "notify",
            "sql",
            "tasks",
        ],
    );
    expect(
        "adminNodes",
        vec![
            "create",
            "delete",
            "get",
            "getById",
            "getChildren",
            "query",
            "update",
            "updateProperty",
        ],
    );
    expect("adminSql", vec!["execute", "query"]);

    assert_eq!(output["hasResourceClass"], true);
    assert_eq!(output["contextIsObject"], true);
}

/// The error conventions that must NOT throw: failed reads resolve to
/// null / [] and the documented sentinel/error-object returns stay returns.
/// (The throw-on-error conventions are covered by
/// `test_node_write_errors_throw_in_js` and the branches/scheduler tests.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_swallowed_error_conventions_do_not_throw() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            return {
                // failed reads resolve to null / []
                get: raisin.nodes.get("ws", "/missing"),
                getById: raisin.nodes.getById("ws", "missing"),
                query: raisin.nodes.query("ws", {}),
                children: raisin.nodes.getChildren("ws", "/missing"),
                adminGet: raisin.asAdmin().nodes.get("ws", "/missing"),
                adminQuery: raisin.asAdmin().nodes.query("ws", {}),
                // failed sql resolves to {error, rows:[]} / -1
                sqlQuery: raisin.sql.query("SELECT 1"),
                sqlExecute: raisin.sql.execute("UPDATE x"),
                // failed emit resolves to false
                emit: raisin.events.emit("evt", {}),
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("swallowed_errors_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})).with_all_errors("backend down"));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["get"], serde_json::Value::Null);
    assert_eq!(output["getById"], serde_json::Value::Null);
    assert_eq!(output["query"], serde_json::json!([]));
    assert_eq!(output["children"], serde_json::json!([]));
    assert_eq!(output["adminGet"], serde_json::Value::Null);
    assert_eq!(output["adminQuery"], serde_json::json!([]));
    assert_eq!(output["sqlQuery"]["rows"], serde_json::json!([]));
    assert!(
        output["sqlQuery"]["error"]
            .as_str()
            .unwrap()
            .contains("backend down"),
        "sql.query must return the error message, got {:?}",
        output["sqlQuery"]
    );
    assert_eq!(output["sqlExecute"], serde_json::json!(-1));
    assert_eq!(output["emit"], false);
}

/// Conventions that ride on the registry gateway's argument massaging:
/// http.fetch(url, options) reshapes into the registry's request(method, url,
/// options); functions.execute tolerates a missing context; nodes.history is
/// callable (it used to be mapped in the wrapper but had no host function);
/// crypto.uuid returns a UUID string; tasks.create returns the task payload.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_gateway_argument_shapes() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            const get = raisin.http.fetch("https://example.org/x");
            const post = raisin.http.fetch("https://example.org/y", { method: "POST", body: "b" });
            const uuid = raisin.crypto.uuid();
            const hist = raisin.nodes.history("ws", "some-id", 5);
            const noCtx = raisin.functions.execute("/lib/fn", { a: 1 });
            const task = raisin.tasks.create({ title: "t", assignee: "/users/u1" });
            return {
                getStatus: get.status,
                getMethod: get.body.method,
                getUrl: get.body.url,
                postMethod: post.body.method,
                postBody: post.body.options.body,
                uuidType: typeof uuid,
                uuidLen: uuid.length,
                hist: hist,
                noCtxSuccess: noCtx.success === true,
                noCtxArgs: noCtx.arguments,
                taskIsObject: !!task && typeof task === 'object' && !task.error,
            };
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("gateway_shapes_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(result.success, "function errored: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["getStatus"], 200);
    assert_eq!(output["getMethod"], "GET");
    assert_eq!(output["getUrl"], "https://example.org/x");
    assert_eq!(output["postMethod"], "POST");
    assert_eq!(output["postBody"], "b");
    assert_eq!(output["uuidType"], "string");
    assert_eq!(output["uuidLen"], 36);
    assert_eq!(output["hist"], serde_json::json!([]));
    assert_eq!(output["noCtxSuccess"], true);
    assert_eq!(output["noCtxArgs"], serde_json::json!({ "a": 1 }));
    assert_eq!(output["taskIsObject"], true);
}

// ============= Pooled-runtime tenant isolation =============

use super::pool::QuickJsPool;

/// Build a runtime backed by a dedicated pool (independent of the global one)
/// so per-test isolation / recycle / poison assertions are deterministic.
fn pooled_runtime(capacity: usize, recycle_threshold: usize) -> (QuickJsRuntime, Arc<QuickJsPool>) {
    let pool = Arc::new(QuickJsPool::new(capacity, recycle_threshold));
    (QuickJsRuntime::with_pool(pool.clone()), pool)
}

fn plain_metadata(name: &str) -> FunctionMetadata {
    FunctionMetadata::javascript(name)
}

/// Metadata with an explicit memory budget, so a test can put a generous
/// function and a frugal one on the same pooled runtime.
fn metadata_with_memory(name: &str, max_memory_bytes: u64) -> FunctionMetadata {
    let mut m = FunctionMetadata::javascript(name);
    m.resource_limits.max_memory_bytes = max_memory_bytes;
    m
}

/// A pooled runtime that already retains more than a function's budget allows
/// must be discarded at checkout, not handed over and stamped with a ceiling it
/// is already above.
///
/// `set_memory_limit` is runtime-global but the limit is **per function**, so
/// before `checkout` took the limit as an argument a modest function could
/// inherit a runtime a larger one had grown, and fail its first allocation —
/// reported as `Failed to setup JS environment: out of memory`, or, when
/// QuickJS was left in an exception state with no exception object, as the bare
/// `Exception generated by QuickJS`. Intermittent in production because it
/// depended entirely on which runtime the pool handed out.
///
/// This drives the pool directly rather than running two functions, because how
/// much a *function* leaves behind is not deterministic: a dropped context does
/// release its globals, so a JS-level "memory hog" reproduces the inheritance
/// only sometimes. The decision under test is the vet itself, and a budget
/// below any live runtime's baseline exercises it exactly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pool_discards_a_runtime_that_cannot_host_the_requested_limit() {
    // Capacity 1 forces reuse; a recycle threshold well above 3 keeps the
    // use-count recycle from being what rebuilds the runtime.
    let pool = Arc::new(QuickJsPool::new(1, 500));
    let generous = 128 * 1024 * 1024;

    // Warm one runtime and return it healthy.
    pool.checkout(generous).await.unwrap().mark_healthy();
    assert_eq!(pool.created_count(), 1, "first checkout builds a runtime");

    // Same generous budget: the warm runtime is comfortably under it, so it is
    // reused. Without this half the vet could be "discard everything", which
    // would pass the assertion below while destroying the pool.
    pool.checkout(generous).await.unwrap().mark_healthy();
    assert_eq!(
        pool.created_count(),
        1,
        "a runtime well under the budget must be reused"
    );

    // A budget below what any live QuickJS runtime retains at rest. The warm
    // runtime cannot host it, so it must be dropped and a fresh one built.
    pool.checkout(4 * 1024).await.unwrap().mark_healthy();
    assert_eq!(
        pool.created_count(),
        2,
        "a runtime that cannot host the budget must be discarded, not handed over"
    );
}

/// The vetting must not throw away perfectly good runtimes: two ordinary
/// executions at the same ordinary budget must share one runtime, or the pool
/// stops being a pool and every execution pays a cold start — the RSS-leak
/// regression it exists to prevent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pool_still_reuses_a_runtime_for_ordinary_functions() {
    let (runtime, pool) = pooled_runtime(1, 500);
    let code = "function handler(input) { return { ok: true }; }";

    for tenant in ["t1", "t2"] {
        let r = runtime
            .execute(
                code,
                "handler",
                ExecutionContext::new(tenant, "repo1", "main", "u")
                    .with_input(serde_json::json!({})),
                &metadata_with_memory(tenant, 128 * 1024 * 1024),
                Arc::new(MockFunctionApi::new(serde_json::json!({}))),
                HashMap::new(),
            )
            .await
            .unwrap();
        assert!(r.success, "{tenant} errored: {:?}", r.error);
    }

    assert_eq!(
        pool.created_count(),
        1,
        "an ordinary execution must reuse the warm runtime, not force a rebuild"
    );
}

/// Globals set by one tenant must never be visible to the next execution that
/// reuses the same pooled runtime. Capacity 1 forces reuse of a single runtime
/// across the two sequential executions.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pool_global_state_does_not_leak_between_tenants() {
    let (runtime, _pool) = pooled_runtime(1, 500);

    // Tenant 1 stows a secret on globalThis.
    let set_code = r#"
        function handler(input) {
            globalThis.SECRET = "t1";
            return { secret: globalThis.SECRET };
        }
    "#;
    let context =
        ExecutionContext::new("tenant1", "repo1", "main", "u1").with_input(serde_json::json!({}));
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
    let r1 = runtime
        .execute(
            set_code,
            "handler",
            context,
            &plain_metadata("t1"),
            api,
            HashMap::new(),
        )
        .await
        .unwrap();
    assert!(r1.success);
    assert_eq!(r1.output.unwrap()["secret"], "t1");

    // Tenant 2 reuses the same pooled runtime and must NOT see the secret.
    let read_code = r#"
        function handler(input) {
            return { leaked: typeof globalThis.SECRET !== "undefined" };
        }
    "#;
    let context =
        ExecutionContext::new("tenant2", "repo1", "main", "u2").with_input(serde_json::json!({}));
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
    let r2 = runtime
        .execute(
            read_code,
            "handler",
            context,
            &plain_metadata("t2"),
            api,
            HashMap::new(),
        )
        .await
        .unwrap();
    assert!(r2.success, "second execution errored: {:?}", r2.error);
    assert_eq!(
        r2.output.unwrap()["leaked"],
        false,
        "globalThis leaked across pooled executions"
    );
}

/// A module file supplied by one tenant must not be resolvable by the next
/// execution (which ships no files). The loader is replaced every execution,
/// so the stale path can no longer resolve. Capacity 1 forces runtime reuse.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pool_module_loader_does_not_leak_between_tenants() {
    let (runtime, _pool) = pooled_runtime(1, 500);

    // Tenant 1 runs with a module file.
    let mod_code = r#"
        import { secret } from './util.js';
        export function handler(input) { return { secret }; }
    "#;
    let mut files = HashMap::new();
    files.insert(
        "util.js".to_string(),
        "export const secret = 42;".to_string(),
    );
    let context =
        ExecutionContext::new("tenant1", "repo1", "main", "u1").with_input(serde_json::json!({}));
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
    let r1 = runtime
        .execute(
            mod_code,
            "handler",
            context,
            &plain_metadata("t1"),
            api,
            files,
        )
        .await
        .unwrap();
    assert!(r1.success, "module execution errored: {:?}", r1.error);
    assert_eq!(r1.output.unwrap()["secret"], 42);

    // Tenant 2 imports the SAME path but ships no files: it must fail because
    // the loader was replaced with an empty one.
    let steal_code = r#"
        import { secret } from './util.js';
        export function handler(input) { return { secret }; }
    "#;
    let context =
        ExecutionContext::new("tenant2", "repo1", "main", "u2").with_input(serde_json::json!({}));
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
    let r2 = runtime
        .execute(
            steal_code,
            "handler",
            context,
            &plain_metadata("t2"),
            api,
            HashMap::new(),
        )
        .await
        .unwrap();
    assert!(
        !r2.success,
        "a stale module path from a previous tenant must not resolve, got {:?}",
        r2.output
    );
}

/// A pooled runtime is dropped and rebuilt after `recycle_threshold`
/// executions. With capacity 1 and threshold 2, five executions build three
/// runtimes (uses 1,2 | 1,2 | 1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pool_recycles_runtime_after_threshold() {
    let (runtime, pool) = pooled_runtime(1, 2);

    let code = r#"function handler(input) { return { ok: true }; }"#;
    for _ in 0..5 {
        let context = ExecutionContext::new("tenant1", "repo1", "main", "u1")
            .with_input(serde_json::json!({}));
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
        let r = runtime
            .execute(
                code,
                "handler",
                context,
                &plain_metadata("recycle"),
                api,
                HashMap::new(),
            )
            .await
            .unwrap();
        assert!(r.success);
    }

    assert_eq!(
        pool.created_count(),
        3,
        "5 executions with recycle threshold 2 should have built 3 runtimes"
    );
}

/// An execution that throws poisons its runtime: it is dropped rather than
/// returned to the pool, so the next execution must build a fresh one (the
/// created-runtime counter increases). The same drop-on-return path covers
/// timeouts and interrupts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pool_poisons_runtime_on_error() {
    let (runtime, pool) = pooled_runtime(1, 500);

    // First clean execution builds runtime #1 and returns it to the pool.
    let ok_code = r#"function handler(input) { return { ok: true }; }"#;
    let context =
        ExecutionContext::new("t", "repo1", "main", "u").with_input(serde_json::json!({}));
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
    let r1 = runtime
        .execute(
            ok_code,
            "handler",
            context,
            &plain_metadata("ok"),
            api,
            HashMap::new(),
        )
        .await
        .unwrap();
    assert!(r1.success);
    assert_eq!(pool.created_count(), 1);

    // A throwing execution must NOT return its runtime to the pool.
    let throw_code = r#"function handler(input) { throw new Error("boom"); }"#;
    let context =
        ExecutionContext::new("t", "repo1", "main", "u").with_input(serde_json::json!({}));
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
    let r2 = runtime
        .execute(
            throw_code,
            "handler",
            context,
            &plain_metadata("throw"),
            api,
            HashMap::new(),
        )
        .await
        .unwrap();
    assert!(!r2.success, "throwing function must fail");
    // Still 1 created: the poisoned runtime was dropped, none created yet.
    assert_eq!(pool.created_count(), 1);

    // The next execution has no idle runtime (the poisoned one was discarded),
    // so it must build a fresh one.
    let context =
        ExecutionContext::new("t", "repo1", "main", "u").with_input(serde_json::json!({}));
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
    let r3 = runtime
        .execute(
            ok_code,
            "handler",
            context,
            &plain_metadata("ok2"),
            api,
            HashMap::new(),
        )
        .await
        .unwrap();
    assert!(r3.success);
    assert_eq!(
        pool.created_count(),
        2,
        "a fresh runtime must be built after the previous one was poisoned"
    );
}

/// Many tenants executing concurrently must all succeed and never observe
/// another tenant's data — exclusive checkout guarantees no runtime is shared.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_pool_concurrent_tenants_do_not_cross() {
    // Shared runtime facade backed by the global pool; each execution gets an
    // exclusive runtime for its duration.
    let runtime = Arc::new(QuickJsRuntime::new());

    let code = r#"
        function handler(input) {
            // Echo the caller's tenant id back after some busy work.
            let acc = 0;
            for (let i = 0; i < 5000; i++) { acc = (acc + i) % 7; }
            return { tenant: input.tenant, acc };
        }
    "#;

    let mut handles = Vec::new();
    for i in 0..16u32 {
        let runtime = runtime.clone();
        handles.push(tokio::spawn(async move {
            let tenant = format!("tenant-{}", i);
            let context = ExecutionContext::new(&tenant, "repo1", "main", "u")
                .with_input(serde_json::json!({ "tenant": tenant }));
            let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
            let r = runtime
                .execute(
                    code,
                    "handler",
                    context,
                    &plain_metadata("conc"),
                    api,
                    HashMap::new(),
                )
                .await
                .unwrap();
            (tenant, r)
        }));
    }

    for handle in handles {
        let (tenant, r) = handle.await.unwrap();
        assert!(r.success, "tenant {} errored: {:?}", tenant, r.error);
        assert_eq!(
            r.output.unwrap()["tenant"],
            tenant,
            "a concurrent execution returned another tenant's id"
        );
    }
}

fn rss_kb() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// Repro for the production memory leak: a fresh QuickJsRuntime per
/// execution (exactly what `FunctionExecutor::new()` per job does).
#[tokio::test]
#[ignore]
async fn leak_fresh_runtime_per_execution() {
    let code = r#"
        function handler(input) {
            let s = JSON.stringify(input);
            return { ok: true, len: s.length };
        }
    "#;
    let base = rss_kb();
    for i in 0..2001u32 {
        let runtime = QuickJsRuntime::new();
        let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
            .with_input(serde_json::json!({"i": i, "msg": "hello world"}));
        let metadata = FunctionMetadata::javascript("leaktest");
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
        let result = runtime
            .execute(code, "handler", context, &metadata, api, HashMap::new())
            .await
            .unwrap();
        assert!(result.success);
        if i % 200 == 0 {
            println!(
                "fresh-runtime iter={} rss={} kB (delta {} kB)",
                i,
                rss_kb(),
                rss_kb() as i64 - base as i64
            );
        }
    }
}

/// Control: one shared runtime, fresh context per execution.
#[tokio::test]
#[ignore]
async fn leak_shared_runtime_per_execution() {
    let code = r#"
        function handler(input) {
            let s = JSON.stringify(input);
            return { ok: true, len: s.length };
        }
    "#;
    let runtime = QuickJsRuntime::new();
    let base = rss_kb();
    for i in 0..2001u32 {
        let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
            .with_input(serde_json::json!({"i": i, "msg": "hello world"}));
        let metadata = FunctionMetadata::javascript("leaktest");
        let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));
        let result = runtime
            .execute(code, "handler", context, &metadata, api, HashMap::new())
            .await
            .unwrap();
        assert!(result.success);
        if i % 200 == 0 {
            println!(
                "shared-runtime iter={} rss={} kB (delta {} kB)",
                i,
                rss_kb(),
                rss_kb() as i64 - base as i64
            );
        }
    }
}

/// An error carrying a `code` property must surface that code in the message.
///
/// Adapters signal outcomes with the reserved-code contract
/// (`var e = new Error(msg); e.code = "auth_expired"`), and the engine
/// classifies on the message string. The exception formatter read only
/// `message()`, so every coded error arrived as a generic transient failure:
/// expired credentials were retried instead of pausing the mount, misconfigured
/// mounts hammered the provider forever, and an expired sync cursor was retried
/// instead of triggering a resync. Production hit the last of those.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_thrown_error_code_reaches_the_engine() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            var e = new Error("Microsoft Graph rejected the access token");
            e.code = "auth_expired";
            throw e;
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("coded_error_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(!result.success, "a thrown error must fail the function");
    let err = result.error.expect("error must be reported");
    assert!(
        err.message.contains("auth_expired"),
        "the reserved code must reach the engine, got: {}",
        err.message
    );
    // The human-readable text is preserved, not replaced.
    assert!(
        err.message.contains("rejected the access token"),
        "the original message must survive, got: {}",
        err.message
    );
}

/// The SAME must hold for an `async` handler, whose rejection takes a different
/// code path inside the runtime.
///
/// That path used to carry its own copy of the formatting logic. Fixing only the
/// synchronous one would have covered today's (synchronous) adapters and
/// silently missed every async function — the mirrored-code-paths trap.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_thrown_error_code_reaches_the_engine_from_async() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        async function handler(input) {
            var e = new Error("provider refused the stored cursor");
            e.code = "cursor_invalid";
            throw e;
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("coded_error_async_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    assert!(!result.success, "a rejected async handler must fail");
    let err = result.error.expect("error must be reported");
    assert!(
        err.message.contains("cursor_invalid"),
        "async rejections must carry the code too, got: {}",
        err.message
    );
}

/// An error WITHOUT a code is byte-identical to before — no stray suffix.
///
/// This is the backward-compatibility guarantee: only coded errors change, and
/// every consumer matches with `contains`, so nothing else shifts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_uncoded_error_message_is_unchanged() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            throw new Error("plain failure");
        }
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("uncoded_error_test");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .unwrap();

    let err = result.error.expect("error must be reported");
    assert!(err.message.contains("plain failure"));
    assert!(
        !err.message.contains("[code="),
        "an uncoded error must not gain a code suffix, got: {}",
        err.message
    );
}

// ============================================================================
// CommonJS in script mode
// ============================================================================
//
// A function with no `import` / `export` runs as a plain SCRIPT, not an ES6
// module. `module` is not a thing in a script, so the CommonJS spelling every
// Node-shaped example ends with — `module.exports = { handler }` — threw a
// ReferenceError DURING EVAL, before the handler was ever called.
//
// The failure was invisible in the worst way: the function declared its handler
// at top level and looked perfectly ordinary, and the error named the last line
// rather than the missing feature. Three built-in functions shipped like this,
// `send-magic-link` among them — which is to say passwordless sign-in could not
// send at all, whatever the email configuration said.

/// The exact shape that was broken: a top-level handler plus a trailing
/// `module.exports`.
#[tokio::test]
async fn a_script_may_end_with_module_exports() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) {
            return { ok: true, got: input.value };
        }

        module.exports = { handler };
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({"value": 42}));
    let metadata = FunctionMetadata::javascript("commonjs_tail");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .expect("module.exports must not be a syntax error in script mode");

    assert!(result.success, "error: {:?}", result.error);
    assert_eq!(result.output.unwrap()["got"], 42);
}

/// And the handler may exist ONLY on `module.exports`, which is how a function
/// written as a Node module looks when nothing is declared at top level.
#[tokio::test]
async fn a_handler_reachable_only_through_module_exports_is_found() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        module.exports.handler = function (input) {
            return { ok: true, got: input.value };
        };
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({"value": 7}));
    let metadata = FunctionMetadata::javascript("commonjs_only");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .expect("an export-only handler must be found");

    assert!(result.success, "error: {:?}", result.error);
    assert_eq!(result.output.unwrap()["got"], 7);
}

/// The bare `exports` alias works too — `exports.handler = …` is the other half
/// of the CommonJS idiom, and it must reference the SAME object as
/// `module.exports` or one of the two spellings silently exports nothing.
#[tokio::test]
async fn the_bare_exports_alias_is_the_same_object() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        exports.handler = function (input) {
            return { same: module.exports.handler === exports.handler, got: input.value };
        };
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({"value": 1}));
    let metadata = FunctionMetadata::javascript("commonjs_alias");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .expect("the exports alias must work");

    assert!(result.success, "error: {:?}", result.error);
    let output = result.output.unwrap();
    assert_eq!(output["same"], true);
    assert_eq!(output["got"], 1);
}

/// A top-level declaration still WINS over `module.exports`, so nothing that
/// worked before changes meaning. The two disagree only in code that exports
/// something other than what it declared, and the global is what the old
/// lookup found.
#[tokio::test]
async fn a_top_level_declaration_still_wins() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        function handler(input) { return { from: "global" }; }
        module.exports = { handler: function (input) { return { from: "exports" }; } };
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("commonjs_precedence");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let result = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await
        .expect("executes");

    assert_eq!(result.output.unwrap()["from"], "global");
}

/// A genuinely missing entrypoint must still be an error, and must still say
/// so — the fallback must not turn "not found" into a confusing type error.
#[tokio::test]
async fn a_missing_entrypoint_is_still_reported() {
    let runtime = QuickJsRuntime::new();

    let code = r#"
        module.exports = { somethingElse: function () { return 1; } };
    "#;

    let context = ExecutionContext::new("tenant1", "repo1", "main", "test-user")
        .with_input(serde_json::json!({}));
    let metadata = FunctionMetadata::javascript("commonjs_missing");
    let api = Arc::new(MockFunctionApi::new(serde_json::json!({})));

    let err = runtime
        .execute(code, "handler", context, &metadata, api, HashMap::new())
        .await;

    let reported = match err {
        Err(e) => e.to_string(),
        Ok(r) => r
            .error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no error reported".to_string()),
    };
    assert!(
        reported.contains("handler"),
        "the error must name the entrypoint it looked for: {reported}"
    );
}
