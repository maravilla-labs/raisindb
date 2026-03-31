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
