// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The `ai-tools` retrieval component — `/lib/raisin/ai/ask` and
//! `/lib/raisin/ai/search-documents` — run for real, through this runtime.
//!
//! The component's own suite (`tooling/ai-tools-rag`, `cargo test`) covers the
//! logic natively against a fake host. This file covers what only the real
//! artifact can: that the COMMITTED `main.wasm` is the one the sources build
//! (it is embedded in the server, and CI cannot rebuild it), that it links
//! against this host, and that its host calls go through the gateway with the
//! shapes the retrieval code expects.
//!
//! The latency benchmark at the bottom is `#[ignore]`d:
//!
//! ```text
//! cargo test -p raisin-functions --lib --features wasm --release \
//!     tests_ai_tools_rag -- --ignored --nocapture
//! ```
//!
//! Set `AI_TOOLS_RAG_BENCH_JS` to a directory holding the former JavaScript
//! implementation (`ask/index.js`, `search-documents/index.js`,
//! `agent-shared/*.js`, e.g. extracted with `git show`) to add the QuickJS arm.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::WasmRuntime;
use crate::api::MockFunctionApi;
use crate::runtime::{FunctionRuntime, QuickJsRuntime};
use crate::types::{
    ExecutionContext, ExecutionResult, FunctionCode, FunctionMetadata, ResourceLimits,
};

const RAG_WASM: &[u8] = include_bytes!(
    "../../../../../builtin-packages/ai-tools/content/functions/lib/raisin/ai/ask/main.wasm"
);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// FNV-1a 64 over `src/*.rs` in name order, name then contents, CR skipped —
/// the same fingerprint `tooling/ai-tools-rag/build.rs` stamps into the
/// component.
fn source_fingerprint(dir: &Path) -> String {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".rs"))
        .collect();
    names.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for name in &names {
        let bytes = std::fs::read(dir.join(name)).expect("read source");
        for b in name.as_bytes().iter().chain(bytes.iter()) {
            if *b == b'\r' {
                continue;
            }
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

fn limits() -> ResourceLimits {
    ResourceLimits {
        timeout_ms: 60_000,
        max_memory_bytes: 128 * 1024 * 1024,
        max_instructions: None,
        max_stack_bytes: 1024 * 1024,
    }
}

async fn run_wasm(handler: &str, input: Value, api: Arc<MockFunctionApi>) -> ExecutionResult {
    let context = ExecutionContext::new("tenant1", "repo1", "main", "visitor").with_input(input);
    WasmRuntime::new()
        .execute(
            &FunctionCode::from(RAG_WASM.to_vec()),
            handler,
            context,
            &FunctionMetadata::wasm("ai_tools_rag").with_resource_limits(limits()),
            api,
            HashMap::new(),
        )
        .await
        .expect("the runtime itself must not fail")
}

// ---------------------------------------------------------------------------
// Fixture: the "wer ist der CEO?" case from a production-like site
// ---------------------------------------------------------------------------

fn chunk_row(
    ws: &str,
    path: &str,
    id: &str,
    node_type: &str,
    file_type: Option<&str>,
    idx: i64,
    text: &str,
) -> Value {
    json!({
        "node_id": id, "path": path, "name": id, "node_type": node_type, "workspace_id": ws,
        "score": 0.03, "fulltext_rank": null, "vector_rank": 1, "chunk_index": idx,
        "chunk_text": text, "chunk_text_source": "exact",
        "title": id.replace('-', " "), "file_type": file_type, "url": null,
    })
}

/// A realistic candidate window: what the engine's vector leg ranks first for
/// a short question (three portraits and a garbled PDF), the page that
/// answers, and filler passages to the requested size.
fn hybrid_rows(n: usize) -> Vec<Value> {
    let mut rows = vec![
        chunk_row("assets", "/bap/team/ceo.jpg", "ceo-portrait", "raisin:Asset", Some("image/jpeg"), 0, "Portrait CEO"),
        chunk_row("assets", "/bap/team/cfo.jpg", "cfo-portrait", "raisin:Asset", Some("image/jpeg"), 0, "Portrait CFO"),
        chunk_row("assets", "/bap/presse/vorstand.png", "board", "raisin:Asset", Some("image/png"), 0, "Geschäftsleitung"),
        chunk_row("assets", "/bap/docs/bericht.pdf", "bericht", "raisin:Asset", Some("application/pdf"), 4, "G e s c h ä f t s b e r i c h t 2 0 2 5 ..."),
        chunk_row("stories", "/bap/unternehmen/geschaeftsleitung", "geschaeftsleitung", "studio:Page", None, 0,
            "Die Geschäftsleitung der Baden-Airpark GmbH: Max Muster, Geschäftsführer, leitet das Unternehmen seit 2021."),
        chunk_row("stories", "/demo/ceo", "demo-ceo", "studio:Page", None, 0, "Demo: Jane Doe, CEO of Example Inc."),
    ];
    let filler = "Der Flughafen bietet Parkplätze, Gastronomie und Reisebüros. ".repeat(16);
    let mut i = 0;
    while rows.len() < n {
        rows.push(chunk_row(
            "stories",
            &format!("/bap/seite-{i}"),
            &format!("seite-{i}"),
            "studio:Page",
            None,
            0,
            &filler,
        ));
        i += 1;
    }
    rows.truncate(n);
    rows
}

/// The window size a search leg asked for: `HYBRID_SEARCH($1, <n>, …`.
fn window_of(sql: &str) -> usize {
    sql.split("HYBRID_SEARCH($1, ")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(8)
}

/// The engine, as far as retrieval can tell: the main leg answers the window,
/// the lexical-only expansion leg finds the news item the vector leg missed
/// (no chunk text: a lexical-only hit), and a batched node read answers it.
fn engine(sql: &str, params: &[Value]) -> raisin_error::Result<Value> {
    if sql.contains("HYBRID_SEARCH") && sql.contains("vector_weight => 0") {
        return Ok(json!([{
            "node_id": "news-gf", "path": "/bap/news/neuer-geschaeftsfuehrer", "name": "neuer-geschaeftsfuehrer",
            "node_type": "studio:Page", "workspace_id": "stories", "score": 0.02,
            "fulltext_rank": 1, "vector_rank": null, "chunk_index": null,
            "chunk_text": null, "chunk_text_source": "unavailable", "title": "Neuer Geschäftsführer", "file_type": null,
        }]));
    }
    if sql.contains("HYBRID_SEARCH") {
        return Ok(Value::Array(hybrid_rows(window_of(sql))));
    }
    if sql.contains("WHERE path IN") {
        assert!(
            params
                .iter()
                .any(|p| p == "/bap/news/neuer-geschaeftsfuehrer"),
            "{params:?}"
        );
        return Ok(
            json!([{ "path": "/bap/news/neuer-geschaeftsfuehrer", "properties": {
                "title": "Neuer Geschäftsführer",
                "content": [{ "element_type": "studio:Text", "body": "Max Muster übernimmt als Geschäftsführer die Leitung des Baden-Airpark." }],
            }}]),
        );
    }
    Ok(json!([]))
}

fn model(request: &Value) -> raisin_error::Result<Value> {
    let system = request["messages"][0]["content"].as_str().unwrap_or("");
    let content = if system.contains("You widen a search query") {
        "{\"terms\": [\"Geschäftsführer\"]}"
    } else if system.contains("You judge whether") {
        "{\"sufficient\": true, \"rewrite\": \"\"}"
    } else if system.contains("You check a drafted answer") {
        "{\"sentences\": [{\"n\": 1, \"supported\": true}]}"
    } else {
        "Geschäftsführer ist Max Muster [1]."
    };
    Ok(json!({ "content": content, "model": "stub:model" }))
}

fn fixture_api() -> Arc<MockFunctionApi> {
    Arc::new(
        MockFunctionApi::new(json!({}))
            .with_sql_responder(engine)
            .with_completion_responder(model),
    )
}

const CEO_INPUT: fn() -> Value = || {
    json!({
        "question": "wer ist der CEO?",
        "workspaces": ["stories", "assets"],
        "paths": ["/bap"],
        "locale": "de",
        "base_language": "de",
    })
};

// ---------------------------------------------------------------------------
// The artifact
// ---------------------------------------------------------------------------

/// The committed component was built from the committed sources. CI has no
/// wasm toolchain, so without this an edit to `tooling/ai-tools-rag/src`
/// without `make ai-tools-rag` would ship the old behaviour in the server.
#[tokio::test]
async fn the_committed_component_matches_its_sources() {
    let result = run_wasm("default", json!({}), fixture_api()).await;
    assert!(result.success, "{:?}", result.error);
    let reported = result.output.unwrap()["source_hash"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let expected = source_fingerprint(&repo_root().join("tooling/ai-tools-rag/src"));
    assert_eq!(
        reported, expected,
        "builtin-packages/ai-tools/.../ask/main.wasm is stale: rebuild it with `make ai-tools-rag`"
    );
}

#[tokio::test]
async fn search_documents_scopes_filters_and_keeps_lexical_hits() {
    let api = fixture_api();
    let result = run_wasm(
        "search-documents",
        json!({ "query": "CEO", "workspaces": ["stories", "assets"], "paths": ["/bap"], "expansions": ["Geschäftsführer"] }),
        api.clone(),
    )
    .await;
    assert!(result.success, "{:?}", result.error);
    let out = result.output.unwrap();
    let paths: Vec<&str> = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap())
        .collect();

    assert!(
        !paths
            .iter()
            .any(|p| p.ends_with(".jpg") || p.ends_with(".png")),
        "images are not passages: {paths:?}"
    );
    assert!(
        !paths.contains(&"/demo/ceo"),
        "outside the path scope: {paths:?}"
    );
    assert!(
        paths.contains(&"/bap/unternehmen/geschaeftsleitung"),
        "{paths:?}"
    );
    assert!(
        paths.contains(&"/bap/news/neuer-geschaeftsfuehrer"),
        "the lexical-only hit is kept: {paths:?}"
    );

    let news = out["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == "/bap/news/neuer-geschaeftsfuehrer")
        .unwrap();
    assert!(news["text"]
        .as_str()
        .unwrap()
        .contains("Max Muster übernimmt"));
    assert_eq!(news["workspace"], "stories");
    assert_eq!(news["matched"], "text");

    let sql: Vec<Value> = api.sql_queries();
    let main = sql[0]["sql"].as_str().unwrap();
    assert!(
        main.contains("HYBRID_SEARCH($1, 40, workspaces => $2, granularity => 'chunk')"),
        "{main}"
    );
    assert!(
        main.contains("WHERE ((path = $3 OR path LIKE $4))"),
        "{main}"
    );
    assert_eq!(sql[0]["params"][1], "stories, assets");
}

#[tokio::test]
async fn ask_answers_with_linkable_citations() {
    let result = run_wasm("ask", CEO_INPUT(), fixture_api()).await;
    assert!(result.success, "{:?}", result.error);
    let out = result.output.unwrap();
    assert_eq!(out["grounded"], true);
    assert_eq!(out["model"], "stub:model");
    assert_eq!(out["attempts"][0]["expansions"], json!(["Geschäftsführer"]));
    let c = &out["citations"][0];
    for field in [
        "marker",
        "path",
        "workspace",
        "node_type",
        "title",
        "snippet",
    ] {
        assert!(!c[field].is_null(), "citation lacks {field}: {c}");
    }
}

/// "Wem gehört der Flughafen?" on the real site: the fact is only in a PDF
/// ("Die Anteile an der Gesellschaft liegen zu 66% bei …"), the vector leg
/// finds a directions page full of town names, and the model once answered by
/// making the towns the owners. Through the real component: the paraphrase
/// expansion reaches the PDF, and the invented sentence is dropped.
#[tokio::test]
async fn ownership_is_answered_from_the_pdf_and_the_invented_owner_is_dropped() {
    const SHARES: &str = "Die Anteile an der Gesellschaft liegen zu 66% bei der Flughafen Stuttgart GmbH und zu 34% bei der Baden-Airpark Beteiligungsgesellschaft.";
    let api = Arc::new(
        MockFunctionApi::new(json!({}))
            .with_sql_responder(|sql, params| {
                if sql.contains("vector_weight => 0") {
                    let terms = params[0].as_str().unwrap_or("");
                    return Ok(if terms.contains("Anteile") {
                        json!([{ "node_id": "pdf", "path": "/bap/downloads/unternehmen.pdf", "name": "unternehmen.pdf",
                            "node_type": "raisin:Asset", "workspace_id": "assets", "fulltext_rank": 1, "vector_rank": null,
                            "chunk_index": null, "chunk_text": null, "chunk_text_source": "unavailable",
                            "title": "Unternehmensprofil", "file_type": "application/pdf" }])
                    } else {
                        json!([])
                    });
                }
                if sql.contains("HYBRID_SEARCH") {
                    return Ok(json!([chunk_row("stories", "/bap/anfahrt", "anfahrt", "studio:Page", None, 0,
                        "Anfahrt: Der Flughafen liegt zwischen Rheinmünster und Hügelsheim, direkt an der A5.")]));
                }
                Ok(json!([{ "path": "/bap/downloads/unternehmen.pdf", "title": "Unternehmensprofil",
                    "body": format!("Unternehmensprofil\n{SHARES}") }]))
            })
            .with_completion_responder(|request| {
                let system = request["messages"][0]["content"].as_str().unwrap_or("");
                let user = request["messages"][1]["content"].as_str().unwrap_or("");
                let content = if system.contains("You widen a search query") {
                    "{\"terms\": [\"Gesellschafter\", \"Anteile\", \"Beteiligung\"]}".to_string()
                } else if system.contains("You judge whether") {
                    "{\"sufficient\": true}".to_string()
                } else if system.contains("You check a drafted answer") {
                    let listing = user.split("Sentences:\n").nth(1).unwrap_or("");
                    let verdicts: Vec<String> = listing
                        .lines()
                        .enumerate()
                        .map(|(i, l)| format!("{{\"n\": {}, \"supported\": {}}}", i + 1, l.contains("66%") && user.contains(SHARES)))
                        .collect();
                    format!("{{\"sentences\": [{}]}}", verdicts.join(", "))
                } else {
                    "Der Flughafen gehört zu 66% der Flughafen Stuttgart GmbH und zu 34% der Baden-Airpark Beteiligungsgesellschaft [2]. \
                     Eigentümer sind außerdem die Gemeinden Rheinmünster und Hügelsheim [1]."
                        .to_string()
                };
                Ok(json!({ "content": content, "model": "stub:model" }))
            }),
    );
    let result = run_wasm(
        "ask",
        json!({ "question": "Wem gehört der Flughafen?", "workspaces": ["stories", "assets"], "paths": ["/bap"], "base_language": "de" }),
        api,
    )
    .await;
    assert!(result.success, "{:?}", result.error);
    let out = result.output.unwrap();
    assert_eq!(out["grounded"], true);
    assert_eq!(out["verification"], "trimmed");
    let answer = out["answer"].as_str().unwrap();
    assert!(
        answer.contains("66%") && !answer.contains("Gemeinden"),
        "{answer}"
    );
    assert!(out["citations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["path"] == "/bap/downloads/unternehmen.pdf"));
}

#[tokio::test]
async fn an_agent_run_gets_the_tool_result_envelope() {
    let mut input = CEO_INPUT();
    input["__raisin_context"] = json!({ "run_id": "run-1", "operation_id": "op-7" });
    let result = run_wasm("ask", input, fixture_api()).await;
    assert!(result.success, "{:?}", result.error);
    let out = result.output.unwrap();
    assert_eq!(out["envelope"], "raisin.tool-result/1");
    assert_eq!(out["operation_id"], "op-7");
    assert_eq!(out["status"], "succeeded");
    assert_eq!(out["payload"]["grounded"], true);
}

#[tokio::test]
async fn a_bad_input_is_a_function_error_naming_the_field() {
    let result = run_wasm(
        "search-documents",
        json!({ "query": "x", "paths": ["bap"] }),
        fixture_api(),
    )
    .await;
    assert!(!result.success);
    let message = result.error.map(|e| e.message).unwrap_or_default();
    assert!(message.contains("paths"), "{message}");
}

// ---------------------------------------------------------------------------
// Latency: QuickJS (the former implementation) vs this component
// ---------------------------------------------------------------------------

const SAMPLES: usize = 20;

fn js_files(root: &Path) -> HashMap<String, String> {
    let mut files = HashMap::new();
    let shared = root.join("agent-shared");
    for entry in
        std::fs::read_dir(&shared).unwrap_or_else(|e| panic!("read {}: {e}", shared.display()))
    {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".js") {
            files.insert(
                format!("agent-shared/{name}"),
                std::fs::read_to_string(&path).unwrap(),
            );
        }
    }
    files
}

/// What the JavaScript `ask` got back from `functions.call(search-documents)`:
/// the JS search-documents' own mapping of an 8-row window.
fn js_search_results() -> Value {
    let results: Vec<Value> = hybrid_rows(8)
        .into_iter()
        .map(|r| json!({
            "path": r["path"], "node_id": r["node_id"], "title": r["name"], "chunk_index": r["chunk_index"],
            "text": r["chunk_text"], "text_is_exact": true, "score": r["score"],
        }))
        .collect();
    json!({ "results": results, "count": results.len() })
}

async fn time<F, Fut>(label: &str, mut call: F, rows: &mut Vec<String>)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ExecutionResult>,
{
    let started = Instant::now();
    let first = call().await;
    assert!(first.success, "[{label}] {:?}", first.error);
    let cold = started.elapsed();
    let mut samples: Vec<Duration> = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let t = Instant::now();
        let r = call().await;
        samples.push(t.elapsed());
        assert!(r.success, "[{label}] {:?}", r.error);
    }
    samples.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    rows.push(format!(
        "  {label:<44} cold {:>9.3} ms   median {:>8.3} ms   best {:>8.3} ms",
        ms(cold),
        ms(samples[SAMPLES / 2]),
        ms(samples[0])
    ));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark: run with --ignored --nocapture (ideally --release)"]
async fn bench_ai_tools_rag_quickjs_vs_wasm() {
    let mut rows: Vec<String> = Vec::new();
    let search_input = json!({ "query": "wer ist der CEO?", "workspaces": "stories, assets" });
    let mut ask_plain = CEO_INPUT();
    ask_plain["expand"] = json!(false);

    // ---- wasm ----
    time(
        "wasm   search-documents (window 40)",
        || run_wasm("search-documents", search_input.clone(), fixture_api()),
        &mut rows,
    )
    .await;
    time(
        "wasm   ask, stub model, no expansion",
        || run_wasm("ask", ask_plain.clone(), fixture_api()),
        &mut rows,
    )
    .await;
    time(
        "wasm   ask, stub model, with expansion",
        || run_wasm("ask", CEO_INPUT(), fixture_api()),
        &mut rows,
    )
    .await;

    // ---- QuickJS, when the former sources are provided ----
    if let Ok(dir) = std::env::var("AI_TOOLS_RAG_BENCH_JS") {
        let root = PathBuf::from(dir);
        let files = Arc::new(js_files(&root));
        let search_js = std::fs::read_to_string(root.join("search-documents/index.js"))
            .expect("search-documents/index.js");
        let ask_js = std::fs::read_to_string(root.join("ask/index.js")).expect("ask/index.js");
        let run_js = |source: String,
                      input: Value,
                      api: Arc<MockFunctionApi>,
                      files: Arc<HashMap<String, String>>| async move {
            let context =
                ExecutionContext::new("tenant1", "repo1", "main", "visitor").with_input(input);
            QuickJsRuntime::new()
                .execute(
                    &FunctionCode::from(source),
                    "handler",
                    context,
                    &FunctionMetadata::javascript("js").with_resource_limits(limits()),
                    api,
                    (*files).clone(),
                )
                .await
                .expect("quickjs runtime")
        };
        time(
            "quickjs search-documents (limit 8)",
            || {
                run_js(
                    search_js.clone(),
                    search_input.clone(),
                    fixture_api(),
                    files.clone(),
                )
            },
            &mut rows,
        )
        .await;
        let js_ask_api = || {
            Arc::new(
                MockFunctionApi::new(json!({}))
                    .with_completion_responder(model)
                    .with_function_call_responder(|_, _| Ok(js_search_results())),
            )
        };
        time(
            "quickjs ask, stub model (search call stubbed)",
            || {
                run_js(
                    ask_js.clone(),
                    json!({ "question": "wer ist der CEO?", "workspaces": "stories, assets" }),
                    js_ask_api(),
                    files.clone(),
                )
            },
            &mut rows,
        )
        .await;
    } else {
        rows.push(
            "  (QuickJS arm skipped: set AI_TOOLS_RAG_BENCH_JS to the former sources)".to_string(),
        );
    }

    println!("\nai-tools retrieval, {SAMPLES} warm samples after one cold call, mock host (no storage, no model):");
    for r in &rows {
        println!("{r}");
    }
}
