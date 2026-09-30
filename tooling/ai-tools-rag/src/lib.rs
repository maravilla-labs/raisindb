//! The retrieval functions of the `ai-tools` package, as one WebAssembly
//! component.
//!
//! ```text
//! /lib/raisin/ai/ask               handler "ask"               (ask.rs)
//! /lib/raisin/ai/search-documents  handler "search-documents"  (retrieve.rs)
//!                                  handler "default"           fingerprint probe
//! ```
//!
//! They were QuickJS functions. They are Rust now because a website chatbot
//! runs them on every turn, and because `ask` used to reach retrieval through
//! `raisin.functions.call` — a second function execution per attempt. Here
//! retrieval is an in-process call; the paths, the input and the output
//! contract are unchanged, and every new input is optional.
//!
//! Both handlers take a `serde_json::Value` so an unknown or mistyped field is
//! a precise error message rather than a serde decode failure.

pub mod ask;
pub mod backend;
pub mod envelope;
pub mod options;
pub mod retrieve;
pub mod text;

use backend::{Backend, Host};
use options::SearchOptions;
use serde_json::{json, Value};

/// Fingerprint of this crate's `src/`, stamped by `build.rs`. The host-side
/// artifact test compares it with the sources in the checkout, so a committed
/// component that was not rebuilt after an edit fails CI.
pub const SOURCE_HASH: &str = env!("AI_TOOLS_RAG_SOURCE_HASH");

/// `search-documents`: passages, no model call.
pub fn search_documents(b: &dyn Backend, input: &Value) -> Result<Value, String> {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if query.is_empty() {
        return Err("A non-empty `query` is required".into());
    }
    let opts = SearchOptions::parse(input, &query)?;
    let found = retrieve::retrieve(b, &opts)?;
    let results: Vec<Value> = found
        .passages
        .iter()
        .map(|p| p.to_json(&found.terms))
        .collect();
    b.log(&format!(
        "[search-documents] \"{}\" in {} → {} passage(s) ({})",
        query.chars().take(80).collect::<String>(),
        opts.scope,
        results.len(),
        found.mode
    ));
    let mut out = json!({
        "count": results.len(),
        "results": results,
        "mode": found.mode,
        "timings": found.timings,
    });
    if !opts.expansions.is_empty() {
        out["expansions"] = json!(opts.expansions);
    }
    Ok(out)
}

/// Run a tool body, enveloped when (and only when) an AgentRun invoked it.
fn tool(
    input: Value,
    body: fn(&dyn Backend, &Value) -> Result<Value, String>,
    next: fn(&Value) -> Value,
) -> raisin_sdk::Result<Value> {
    let result = body(&Host, &input);
    match envelope::run_operation_id(&input) {
        Some(op) => Ok(match result {
            Ok(v) => {
                let n = next(&v);
                envelope::success(&op, v, n)
            }
            Err(e) => envelope::failure(&op, &e),
        }),
        None => result.map_err(raisin_sdk::Error::Host),
    }
}

fn no_next(_: &Value) -> Value {
    json!([])
}

fn rephrase_when_empty(v: &Value) -> Value {
    match v.get("results").and_then(Value::as_array) {
        Some(r) if r.is_empty() => {
            json!([{ "action": "rephrase_query", "reason": "no passages matched" }])
        }
        _ => json!([]),
    }
}

#[raisin_sdk::handler(name = "ask")]
pub fn ask_handler(input: Value) -> raisin_sdk::Result<Value> {
    tool(input, ask::ask, no_next)
}

#[raisin_sdk::handler(name = "search-documents")]
pub fn search_documents_handler(input: Value) -> raisin_sdk::Result<Value> {
    tool(input, search_documents, rephrase_when_empty)
}

/// Which build of the component answered.
#[raisin_sdk::handler]
pub fn probe(_input: Value) -> raisin_sdk::Result<Value> {
    Ok(json!({
        "component": "ai-tools-rag",
        "version": env!("CARGO_PKG_VERSION"),
        "source_hash": SOURCE_HASH,
        "handlers": ["ask", "search-documents"],
    }))
}

raisin_sdk::export!(ask_handler, search_documents_handler, probe);

#[cfg(test)]
mod tests {
    //! The JavaScript suite of `search-documents/index.test.mjs`, ported.

    use super::*;
    use crate::backend::fake::Fake;

    fn row(id: &str, path: &str, idx: Option<i64>, text: Option<&str>, source: &str) -> Value {
        json!({ "node_id": id, "path": path, "name": id.to_uppercase(), "node_type": "studio:Page",
                "workspace_id": "docs", "score": 0.5, "fulltext_rank": null, "vector_rank": 1,
                "chunk_index": idx, "chunk_text": text, "chunk_text_source": source })
    }

    fn with_rows(rows: Vec<Value>) -> Fake {
        Fake {
            sql_fn: Box::new(move |sql, _| {
                if sql.contains("HYBRID_SEARCH") {
                    Ok(rows.clone())
                } else {
                    Ok(vec![])
                }
            }),
            ..Fake::default()
        }
    }

    #[test]
    fn asks_for_passages_not_documents() {
        let f = with_rows(vec![]);
        search_documents(&f, &json!({ "query": "notice period" })).unwrap();
        assert!(f.sqls()[0].contains("granularity => 'chunk'"));
    }

    #[test]
    fn caller_controlled_values_are_bound_never_interpolated() {
        let f = with_rows(vec![]);
        search_documents(
            &f,
            &json!({ "query": "o'brien", "workspaces": "docs, handbook", "paths": ["/o'brien"] }),
        )
        .unwrap();
        let log = f.sql_log.borrow();
        assert_eq!(log[0].1[..2], [json!("o'brien"), json!("docs, handbook")]);
        assert!(
            !log[0].0.contains("o'brien"),
            "the query text must not be concatenated into the SQL"
        );
    }

    #[test]
    fn the_limit_is_clamped() {
        let f = with_rows(
            (0..80)
                .map(|i| {
                    row(
                        &format!("n{i}"),
                        &format!("/p/{i}"),
                        Some(0),
                        Some("t"),
                        "exact",
                    )
                })
                .collect(),
        );
        let out = search_documents(&f, &json!({ "query": "x", "limit": 5000 })).unwrap();
        assert_eq!(out["count"], json!(50));
        assert!(
            f.sqls()[0].contains("HYBRID_SEARCH($1, 50,"),
            "the window is never below the limit"
        );
    }

    #[test]
    fn defaults_to_every_readable_workspace_spelled_the_one_accepted_way() {
        let f = with_rows(vec![]);
        search_documents(&f, &json!({ "query": "x" })).unwrap();
        assert_eq!(f.sql_log.borrow()[0].1[1], json!("ALL READABLE"));
    }

    #[test]
    fn a_preview_passage_is_reported_as_not_exact() {
        let f = with_rows(vec![
            row(
                "n1",
                "/contracts/msa",
                Some(3),
                Some("Either party may terminate…"),
                "exact",
            ),
            row(
                "n2",
                "/contracts/nda",
                Some(0),
                Some("truncated preview…"),
                "excerpt",
            ),
        ]);
        let out = search_documents(&f, &json!({ "query": "terminate" })).unwrap();
        assert_eq!(out["count"], json!(2));
        assert_eq!(out["results"][0]["text_is_exact"], json!(true));
        assert_eq!(out["results"][1]["text_is_exact"], json!(false));
    }

    #[test]
    fn chunk_index_0_survives() {
        let f = with_rows(vec![row(
            "n1",
            "/notes/one",
            Some(0),
            Some("body"),
            "exact",
        )]);
        let out = search_documents(&f, &json!({ "query": "x" })).unwrap();
        assert_eq!(out["results"][0]["chunk_index"], json!(0));
    }

    #[test]
    fn a_passage_with_no_text_anywhere_is_dropped_rather_than_cited_as_empty() {
        let f = with_rows(vec![row("n1", "/a", Some(1), None, "unavailable")]);
        let out = search_documents(&f, &json!({ "query": "x" })).unwrap();
        assert_eq!(out["count"], json!(0));
        assert_eq!(out["results"], json!([]));
    }

    #[test]
    fn an_empty_query_is_rejected() {
        let f = with_rows(vec![]);
        assert!(search_documents(&f, &json!({ "query": "   " }))
            .unwrap_err()
            .contains("non-empty"));
    }

    #[test]
    fn every_result_carries_the_link_fields() {
        let f = with_rows(vec![row(
            "n1",
            "/contracts/msa",
            Some(3),
            Some("Either party may terminate."),
            "exact",
        )]);
        let out = search_documents(&f, &json!({ "query": "terminate" })).unwrap();
        let r = &out["results"][0];
        assert_eq!(r["workspace"], json!("docs"));
        assert_eq!(r["node_type"], json!("studio:Page"));
        assert_eq!(r["kind"], json!("page"));
        assert_eq!(
            r["title"],
            json!("N1"),
            "no title property: the node name, as before"
        );
        assert_eq!(r["snippet"], json!("Either party may terminate."));
        assert_eq!(out["mode"], json!("hybrid"));
    }
}
