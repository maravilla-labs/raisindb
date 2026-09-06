//! Cross-runtime benchmark against a REAL server: QuickJS vs Starlark vs wasm.
//!
//! Run with:
//!   cargo test -p raisin-server --test all runtime_bench_test -- --ignored --nocapture
//!
//! # How this differs from the unit benchmark
//!
//! `raisin-functions`' `bench_runtimes` runs the same three arms against a mock
//! host, which isolates runtime overhead. This one runs them through the HTTP
//! invoke endpoint against a live server, so every call really reads nodes out
//! of RocksDB and really executes SQL over 25 seeded pages.
//!
//! That makes it the honest answer to "does the runtime choice matter in
//! practice": the storage and SQL work is identical whichever runtime asked for
//! it, so it lands as a constant on every arm and COMPRESSES the ratios the
//! unit benchmark reports. Both numbers are true; they answer different
//! questions.
//!
//! Two timings are collected per arm:
//!   * `wall`   — client-observed round-trip, what a caller actually waits for.
//!   * `server` — the `duration_ms` the invoke response reports, execution only.
//! Their difference is everything outside the runtime — HTTP, auth, node
//! lookup, code load, and the three RocksDB writes the sync invoke path makes
//! per call. Reporting both is what made two ~190 ms bugs visible: they showed
//! up as a gap no arm could explain, since server-side execution stayed flat
//! while the client's wall clock did not. Both are fixed; the gap is now ~2 ms.
//! Keep reporting both — a single number would have hidden them.
//!
//! The `realistic` outputs of all three arms are asserted EQUAL before any
//! timing is printed: arms that disagree are not running the same workload.

#[allow(unused_imports)]
use crate::helpers;
use std::time::{Duration, Instant};

use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::{multipart, Client};
use serde_json::{json, Value};

const REPO: &str = "benchfn";
const TENANT: &str = "default";
const ADMIN_USER: &str = "admin";
const ADMIN_PASS: &str = "Admin12345!@#";
const PORT: u16 = 8113;

/// Seeded `raisin:Page` nodes the functions read and query.
const PAGES: usize = 25;
/// Invocations per arm after the first (cold) one.
const SAMPLES: usize = 15;
/// `compute` iterations. Lower than the unit benchmark's 100k because Starlark
/// walks an AST per iteration and this arm also pays a network round-trip.
const COMPUTE_N: u64 = 50_000;

/// The same three sources the unit benchmark runs — one copy, so the two
/// benchmarks cannot drift into timing different work.
const JS_SOURCE: &str = include_str!("../../../../fixtures/bench-sources/bench.js");
const STARLARK_SOURCE: &str = include_str!("../../../../fixtures/bench-sources/bench.star");
const BENCH_WASM: &[u8] =
    include_bytes!("../../../raisin-functions/src/runtime/wasm/fixtures/bench.wasm");

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Create a node, retrying while the repo's node types are still initializing.
///
/// A fresh repo registers its node types asynchronously, so the first writes
/// after `POST /api/repositories` can fail with
/// `NodeType 'raisin:Folder' not found`. That is a startup race, not a
/// rejection, so it is retried rather than asserted away — every other failure
/// still fails the test, on the last attempt's message.
async fn post_node(client: &Client, base: &str, token: &str, ws: &str, path: &str, node: Value) {
    let mut last = String::new();
    for _ in 0..60 {
        let response = client
            .post(format!(
                "{base}/api/repository/{REPO}/main/head/{ws}/{path}"
            ))
            .bearer_auth(token)
            .json(&json!({ "node": node }))
            .send()
            .await
            .expect("create node request failed");
        if response.status().is_success() {
            return;
        }
        last = response.text().await.unwrap_or_default();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("creating {path}/{} failed: {last}", node["name"]);
}

fn function_node(name: &str, language: &str, entry_file: &str) -> Value {
    json!({
        "name": name,
        "node_type": "raisin:Function",
        "properties": {
            "name": name,
            "title": name,
            "language": language,
            "entry_file": entry_file,
            "execution_mode": "both",
            "enabled": true,
        }
    })
}

/// A code asset carried INLINE, the way `raisindb sync --watch` pushes a text
/// function. The wasm arm cannot use this path — its bytes go up as multipart.
fn code_asset(file_name: &str, source: &str) -> Value {
    json!({
        "name": file_name,
        "node_type": "raisin:Asset",
        "properties": { "title": file_name, "code": source }
    })
}

/// One arm: a language, its two function nodes, and how its code was delivered.
struct Arm {
    label: &'static str,
    realistic: String,
    compute: String,
}

// ---------------------------------------------------------------------------
// Measurement
// ---------------------------------------------------------------------------

/// Invoke a function synchronously, returning its output plus both timings.
async fn invoke(
    client: &Client,
    base: &str,
    token: &str,
    name: &str,
    input: &Value,
) -> (Value, Duration, u64) {
    let started = Instant::now();
    let response = client
        .post(format!("{base}/api/functions/{REPO}/{name}/invoke"))
        .bearer_auth(token)
        .json(&json!({ "input": input, "sync": true, "timeout_ms": 120_000 }))
        .send()
        .await
        .expect("invoke request failed");
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let wall = started.elapsed();

    assert!(
        status.is_success(),
        "invoking {name} returned {status}: {body}"
    );
    let parsed: Value = serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("invoke {name} body was not JSON ({e}): {body}"));
    assert!(
        parsed.get("error").map(|e| e.is_null()).unwrap_or(true),
        "{name} failed: {}",
        parsed["error"]
    );

    let server_ms = parsed
        .get("duration_ms")
        .and_then(|d| d.as_u64())
        .unwrap_or(0);
    (parsed["result"].clone(), wall, server_ms)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Cold invocation, then `SAMPLES` steady-state ones.
async fn measure(
    client: &Client,
    base: &str,
    token: &str,
    name: &str,
    input: &Value,
) -> (Value, f64, f64, f64) {
    let (output, cold, _) = invoke(client, base, token, name, input).await;

    let mut walls = Vec::with_capacity(SAMPLES);
    let mut servers = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let (again, wall, server_ms) = invoke(client, base, token, name, input).await;
        assert_eq!(again, output, "{name} is not deterministic across runs");
        walls.push(ms(wall));
        servers.push(server_ms as f64);
    }
    walls.sort_by(|a, b| a.partial_cmp(b).unwrap());
    servers.sort_by(|a, b| a.partial_cmp(b).unwrap());

    (
        output,
        ms(cold),
        walls[walls.len() / 2],
        servers[servers.len() / 2],
    )
}

// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark: needs a server process; run with --ignored --nocapture"]
async fn compare_runtimes_against_a_real_server() {
    println!("\n🧪 runtime benchmark: real nodes, real SQL");

    let config = ServerConfig::new(PORT).with_rust_log("info,invoke_timing=debug");
    let server = ServerHandle::start(config)
        .await
        .expect("server failed to start");

    // The admin user is created asynchronously during startup - retry auth.
    let mut token = None;
    for _ in 0..30 {
        match authenticate(&server.base_url, TENANT, ADMIN_USER, ADMIN_PASS).await {
            Ok(t) => {
                token = Some(t);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
    let token = token.expect("failed to authenticate within 15s");
    let client = Client::new();
    let base = server.base_url.clone();

    // Repo
    let repo = client
        .post(format!("{base}/api/repositories"))
        .bearer_auth(&token)
        .json(&json!({ "repo_id": REPO, "description": "runtime benchmark" }))
        .send()
        .await
        .expect("create repo failed");
    assert!(repo.status().is_success(), "repo creation failed");

    // Real content: a folder of pages the functions read and query.
    post_node(
        &client,
        &base,
        &token,
        "default",
        "",
        json!({ "name": "pages", "node_type": "raisin:Folder", "properties": { "title": "Pages" } }),
    )
    .await;
    for i in 0..PAGES {
        post_node(
            &client,
            &base,
            &token,
            "default",
            "pages",
            json!({
                "name": format!("page-{i:02}"),
                "node_type": "raisin:Page",
                "properties": { "title": format!("Page {i}") }
            }),
        )
        .await;
    }
    println!("✅ seeded {PAGES} pages");

    // `/lib` is seeded by the functions workspace; wait for it to be readable.
    for _ in 0..60 {
        let r = client
            .get(format!(
                "{base}/api/repository/{REPO}/main/head/functions/lib"
            ))
            .bearer_auth(&token)
            .send()
            .await
            .expect("read /lib failed");
        if r.status().is_success() {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // One function node per (language, workload). The `compute` node reaches
    // the SAME artifact as its `realistic` sibling through a parent-relative
    // entry_file, so each language's code is stored once.
    let arms = vec![
        ("quickjs", "javascript", "index.js", Some(JS_SOURCE)),
        ("starlark", "starlark", "main.star", Some(STARLARK_SOURCE)),
        ("wasm", "wasm", "main.wasm", None),
    ];
    let mut built = Vec::new();
    for (label, language, file, source) in arms {
        let realistic = format!("bench-{label}");
        let compute = format!("bench-{label}-compute");

        post_node(
            &client,
            &base,
            &token,
            "functions",
            "lib",
            function_node(&realistic, language, &format!("{file}:realistic")),
        )
        .await;
        post_node(
            &client,
            &base,
            &token,
            "functions",
            "lib",
            function_node(
                &compute,
                language,
                &format!("../{realistic}/{file}:compute"),
            ),
        )
        .await;

        match source {
            Some(code) => {
                post_node(
                    &client,
                    &base,
                    &token,
                    "functions",
                    &format!("lib/{realistic}"),
                    code_asset(file, code),
                )
                .await;
            }
            None => {
                let form = multipart::Form::new().part(
                    "file",
                    multipart::Part::bytes(BENCH_WASM.to_vec())
                        .file_name("main.wasm")
                        .mime_str("application/wasm")
                        .unwrap(),
                );
                let upload = client
                    .post(format!(
                        "{base}/api/repository/{REPO}/main/head/functions/lib/{realistic}/main.wasm?override_existing=true"
                    ))
                    .bearer_auth(&token)
                    .multipart(form)
                    .send()
                    .await
                    .expect("artifact upload failed");
                assert!(
                    upload.status().is_success(),
                    "artifact upload failed: {}",
                    upload.text().await.unwrap_or_default()
                );
            }
        }

        built.push(Arm {
            label,
            realistic,
            compute,
        });
    }
    println!("✅ {} arms deployed", built.len());

    // -----------------------------------------------------------------
    // CONTROL: a cheap GET that touches NO function, NO runtime and NO job
    // bookkeeping. If this is also ~190 ms then the overhead is transport or
    // client level and nothing in the invoke path can explain it; if it is
    // ~1 ms the overhead really is the job writes. Measuring it is the
    // difference between fixing the right thing and guessing.
    // -----------------------------------------------------------------
    {
        let mut samples = Vec::new();
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let r = client
                .get(format!(
                    "{base}/api/repository/{REPO}/main/head/functions/lib"
                ))
                .bearer_auth(&token)
                .send()
                .await
                .expect("control GET failed");
            assert!(r.status().is_success());
            let _ = r.text().await;
            samples.push(ms(started.elapsed()));
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // Second control: /health is unauthenticated and touches no storage.
        // If it costs the same as the authenticated GET above, the baseline is
        // transport or client, not auth or RLS — which decides where any
        // further latency work belongs.
        let mut health = Vec::new();
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let r = client
                .get(format!("{base}/health"))
                .send()
                .await
                .expect("health GET failed");
            let _ = r.text().await;
            health.push(ms(started.elapsed()));
        }
        health.sort_by(|a, b| a.partial_cmp(b).unwrap());

        println!(
            "\n  control GET  (authed, reads a node): median {:.2} ms, min {:.2} ms\n  \
             control /health (no auth, no storage): median {:.2} ms, min {:.2} ms",
            samples[samples.len() / 2],
            samples[0],
            health[health.len() / 2],
            health[0]
        );
    }

    // -----------------------------------------------------------------
    // realistic — real node read + real SQL
    // -----------------------------------------------------------------
    let realistic_input = json!({
        "workspace": "default",
        "path": "/pages",
        "limit": PAGES,
        "node_type": "raisin:Page",
        "sql": "SELECT id, name FROM 'default' WHERE node_type = $1",
    });

    let mut rows = vec![String::new()];
    let mut outputs: Vec<(&str, Value)> = Vec::new();
    let mut timings: Vec<(&str, f64, f64, f64)> = Vec::new();

    for arm in &built {
        let (output, cold, wall, server) =
            measure(&client, &base, &token, &arm.realistic, &realistic_input).await;
        outputs.push((arm.label, output));
        timings.push((arm.label, cold, wall, server));
    }

    let (first_label, first_output) = &outputs[0];
    for (label, output) in &outputs[1..] {
        assert_eq!(
            output, first_output,
            "realistic: {label} disagreed with {first_label} — the arms are not \
             running the same workload, so timing them is meaningless"
        );
    }

    let render = |title: String, timings: &Vec<(&str, f64, f64, f64)>, rows: &mut Vec<String>| {
        let baseline = timings
            .iter()
            .map(|(_, _, _, s)| *s)
            .fold(f64::INFINITY, f64::min)
            .max(0.001);
        rows.push(title);
        rows.push(format!(
            "    {:<10} {:>10} {:>12} {:>12} {:>8}",
            "runtime", "cold ms", "wall ms", "server ms", "vs best"
        ));
        for (label, cold, wall, server) in timings {
            rows.push(format!(
                "    {:<10} {:>10.1} {:>12.2} {:>12.2} {:>7.1}x",
                label,
                cold,
                wall,
                server,
                server / baseline
            ));
        }
    };

    render(
        format!(
            "\n  realistic — {PAGES} nodes + SQL  (output: {})",
            serde_json::to_string(first_output).unwrap_or_default()
        ),
        &timings,
        &mut rows,
    );

    // -----------------------------------------------------------------
    // compute — CPU only, no host calls
    // -----------------------------------------------------------------
    let compute_input = json!({ "n": COMPUTE_N });
    let mut compute_timings = Vec::new();
    let mut compute_outputs: Vec<(&str, Value)> = Vec::new();
    for arm in &built {
        let (output, cold, wall, server) =
            measure(&client, &base, &token, &arm.compute, &compute_input).await;
        compute_outputs.push((arm.label, output));
        compute_timings.push((arm.label, cold, wall, server));
    }
    let (cf_label, cf_output) = &compute_outputs[0];
    for (label, output) in &compute_outputs[1..] {
        assert_eq!(
            output, cf_output,
            "compute: {label} disagreed with {cf_label}"
        );
    }
    render(
        format!(
            "\n  compute (n={COMPUTE_N}) — no host calls  (output: {})",
            serde_json::to_string(cf_output).unwrap_or_default()
        ),
        &compute_timings,
        &mut rows,
    );

    // -----------------------------------------------------------------
    // CONCURRENCY SWEEP
    //
    // Everything above is SEQUENTIAL — one request at a time — which measures
    // latency and says nothing about throughput. For an HTTP API layer the
    // binding constraint is usually not per-call cost but
    // `RAISIN_MAX_CONCURRENT_FUNCTIONS` (default 15, `runtime/mod.rs`), a
    // process-wide semaphore sized for background jobs. This sweep shows where
    // that ceiling actually bites: req/s should climb with in-flight requests
    // and then flatten, and p99 should start climbing at the same point.
    // -----------------------------------------------------------------
    {
        let wasm_fn = built
            .iter()
            .find(|a| a.label == "wasm")
            .expect("wasm arm exists");
        let mut lines = vec![format!(
            "\n=== concurrency sweep (wasm, realistic, {PAGES} nodes) ===\n    {:>9} {:>10} {:>10} {:>10} {:>10}",
            "in-flight", "req/s", "p50 ms", "p95 ms", "p99 ms"
        )];

        for &inflight in &[1usize, 4, 16, 48] {
            // Enough requests that each level is more than warm-up noise.
            let total = (inflight * 12).max(24);
            let started = Instant::now();
            let mut latencies = Vec::with_capacity(total);

            // Fixed-size wave of concurrent requests, repeated until `total`.
            let mut sent = 0usize;
            while sent < total {
                let batch = inflight.min(total - sent);
                let mut handles = Vec::with_capacity(batch);
                for _ in 0..batch {
                    let client = client.clone();
                    let base = base.clone();
                    let token = token.clone();
                    let name = wasm_fn.realistic.clone();
                    let input = realistic_input.clone();
                    handles.push(tokio::spawn(async move {
                        let t = Instant::now();
                        let r = client
                            .post(format!("{base}/api/functions/{REPO}/{name}/invoke"))
                            .bearer_auth(&token)
                            .json(&json!({ "input": input, "sync": true }))
                            .send()
                            .await
                            .expect("concurrent invoke failed");
                        assert!(r.status().is_success(), "concurrent invoke: {}", r.status());
                        let _ = r.text().await;
                        ms(t.elapsed())
                    }));
                }
                for h in handles {
                    latencies.push(h.await.expect("request task panicked"));
                }
                sent += batch;
            }

            let wall = started.elapsed().as_secs_f64();
            latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let pick = |q: f64| {
                latencies[((latencies.len() as f64 * q) as usize).min(latencies.len() - 1)]
            };
            lines.push(format!(
                "    {:>9} {:>10.1} {:>10.2} {:>10.2} {:>10.2}",
                inflight,
                total as f64 / wall,
                pick(0.50),
                pick(0.95),
                pick(0.99)
            ));
        }
        println!("{}", lines.join("\n"));
        println!(
            "\n  If req/s stops climbing while p99 rises, the limit is\n  \
             RAISIN_MAX_CONCURRENT_FUNCTIONS (default 15), not the runtime."
        );
    }

    // DIAGNOSTIC: re-measure the arm that was sampled FIRST, now that ~96 jobs
    // have been registered. If it is still fast the overhead is arm- or
    // order-specific; if it has become slow, something ACCUMULATES per
    // invocation and that is the thing to fix.
    {
        let first = &built[0];
        let (_, _, wall_again, server_again) =
            measure(&client, &base, &token, &first.realistic, &realistic_input).await;
        rows.push(format!(
            "\n  re-measured {} (first arm, after ~96 jobs): wall {:.2} ms, server {:.2} ms",
            first.label, wall_again, server_again
        ));
    }

    println!(
        "\n=== real server ({SAMPLES} warm samples, {PAGES} nodes) ==={}",
        rows.join("\n")
    );
    println!(
        "\n  wall - server = everything the runtime does not control: HTTP, auth and\n  \
         the node lookup. It was ~190 ms until three fixes landed: a validated\n  \
         name->path memo for the function lookup (it loaded EVERY function node\n  \
         per call: 170 ms once the builtin packages are installed), a memo on\n  \
         tenant NodeType init (it re-ran, and re-failed, on every request: 20 ms),\n  \
         and dropping job bookkeeping from sync invocations (six storage writes\n  \
         per API call). It is now ~2 ms."
    );
}
