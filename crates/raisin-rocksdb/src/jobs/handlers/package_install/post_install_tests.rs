// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The `post_install` phase, driven through the real install job entry point
//! (`handle`), with a stand-in function worker that completes or fails the
//! queued `FunctionExecution` jobs.

use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobContext, JobInfo, JobStatus, JobType};
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{BranchRepository, NodeRepository, RepositoryManagementRepository, Storage};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use super::handler::PackageInstallHandler;
use crate::jobs::AUTH_CONTEXT_KEY;
use crate::RocksDBStorage;

const TENANT: &str = "default";
const REPO: &str = "testrepo";
const PKG_ID: &str = "package-hooks";

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
    /// Function paths in the order the stand-in worker saw them.
    ran: Arc<Mutex<Vec<String>>>,
    /// Contexts of the queued function jobs, in order.
    contexts: Arc<Mutex<Vec<JobContext>>>,
}

async fn setup() -> Env {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    storage
        .repository_management()
        .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    for branch in ["main", "feature"] {
        storage
            .branches()
            .create_branch(TENANT, REPO, branch, "test", None, None, false, false)
            .await
            .unwrap();
        raisin_core::nodetype_init::init_repository_nodetypes(
            storage.clone(),
            TENANT,
            REPO,
            branch,
        )
        .await
        .unwrap();
    }
    raisin_core::workspace_init::init_repository_workspaces(storage.clone(), TENANT, REPO)
        .await
        .unwrap();

    write(&storage, "main", "functions", folder("/lib")).await;
    for name in ["a", "b", "fail", "warn"] {
        write(
            &storage,
            "main",
            "functions",
            function(&format!("/lib/{name}")),
        )
        .await;
    }
    write(&storage, "main", "packages", package_node()).await;

    let env = Env {
        _dir: dir,
        storage,
        ran: Arc::new(Mutex::new(Vec::new())),
        contexts: Arc::new(Mutex::new(Vec::new())),
    };
    spawn_function_worker(&env);
    env
}

async fn write(storage: &Arc<RocksDBStorage>, branch: &str, ws: &str, node: Node) {
    let tx = storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(branch).unwrap();
    tx.set_actor("test").unwrap();
    tx.set_content_import(true).unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.upsert_node(ws, &node).await.unwrap();
    tx.commit().await.unwrap();
}

fn node(path: &str, node_type: &str, props: &[(&str, PropertyValue)]) -> Node {
    Node {
        id: nanoid::nanoid!(),
        node_type: node_type.to_string(),
        name: path.rsplit('/').next().unwrap().to_string(),
        path: path.to_string(),
        properties: props
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        ..Default::default()
    }
}

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn folder(path: &str) -> Node {
    node(path, "raisin:Folder", &[])
}

fn function(path: &str) -> Node {
    node(
        path,
        "raisin:Function",
        &[("title", s(path)), ("language", s("javascript"))],
    )
}

fn package_node() -> Node {
    let mut n = node(
        "/hooks",
        "raisin:Package",
        &[
            ("name", s("hooks")),
            ("version", s("1.0.0")),
            (
                "resource",
                PropertyValue::Object(HashMap::from([("key".to_string(), s("k"))])),
            ),
        ],
    );
    n.id = PKG_ID.to_string();
    n
}

/// Stand-in for the Realtime pool: completes every queued function job except
/// `/lib/fail`, which it fails.
fn spawn_function_worker(env: &Env) {
    let registry = env.storage.job_registry().clone();
    let data_store = env.storage.job_data_store().clone();
    let ran = env.ran.clone();
    let contexts = env.contexts.clone();
    tokio::spawn(async move {
        loop {
            for job in registry.list_jobs().await {
                let JobType::FunctionExecution { function_path, .. } = &job.job_type else {
                    continue;
                };
                if job.status != JobStatus::Scheduled {
                    continue;
                }
                ran.lock().unwrap().push(function_path.clone());
                if let Ok(Some(ctx)) = data_store.get(TENANT, &job.id) {
                    contexts.lock().unwrap().push(ctx);
                }
                registry.mark_running(&job.id).await.unwrap();
                if function_path == "/lib/fail" {
                    registry
                        .mark_failed(&job.id, "boom: store already half set up".into())
                        .await
                        .unwrap();
                } else {
                    if function_path == "/lib/warn" {
                        // What the function-execution handler stores as the
                        // job result: a FunctionExecutionResult whose `result`
                        // is the function's return value.
                        registry
                            .set_result(
                                &job.id,
                                serde_json::json!({
                                    "execution_id": "x",
                                    "success": true,
                                    "result": {
                                        "warnings": [
                                            "self-registration closed",
                                            { "message": "audiences:/speakers deferred" },
                                            "w".repeat(400)
                                        ],
                                        "seeded": 3
                                    },
                                    "error": null,
                                    "duration_ms": 5,
                                    "logs": []
                                }),
                            )
                            .await
                            .unwrap();
                    }
                    registry.mark_completed(&job.id).await.unwrap();
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
}

fn package_zip(post_install: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buf));
        zip.start_file("manifest.yaml", SimpleFileOptions::default())
            .unwrap();
        write!(zip, "name: hooks\nversion: 1.0.0\n{post_install}").unwrap();
        zip.finish().unwrap();
    }
    buf
}

async fn install(
    env: &Env,
    branch: &str,
    mode: &str,
    post_install: &str,
) -> raisin_error::Result<Option<serde_json::Value>> {
    let zip = package_zip(post_install);
    let handler =
        PackageInstallHandler::new(env.storage.clone(), env.storage.job_registry().clone())
            .with_job_data_store(env.storage.job_data_store().clone())
            .with_post_install_hook_timeout(Duration::from_secs(20))
            .with_binary_callback(Arc::new(move |_key| {
                let zip = zip.clone();
                Box::pin(async move { Ok(zip) })
            }));
    let job_type = JobType::PackageInstall {
        package_name: "hooks".into(),
        package_version: "1.0.0".into(),
        package_node_id: PKG_ID.into(),
    };
    let job_id = env
        .storage
        .job_registry()
        .register_job(job_type, TENANT.into(), None, None, Some(0))
        .await
        .unwrap();
    let job: JobInfo = env
        .storage
        .job_registry()
        .get_job_info(&job_id)
        .await
        .unwrap();
    let context = JobContext {
        tenant_id: TENANT.into(),
        repo_id: REPO.into(),
        branch: branch.into(),
        workspace_id: "packages".into(),
        revision: raisin_hlc::HLC::now(),
        metadata: HashMap::from([
            ("resource_key".to_string(), serde_json::json!("k")),
            ("install_mode".to_string(), serde_json::json!(mode)),
        ]),
    };
    handler.handle(&job, &context).await
}

async fn package_props(env: &Env) -> HashMap<String, PropertyValue> {
    env.storage
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, "main", "packages"),
            PKG_ID,
            None,
        )
        .await
        .unwrap()
        .expect("package node")
        .properties
}

fn statuses(props: &HashMap<String, PropertyValue>) -> Vec<(String, String)> {
    let Some(PropertyValue::Array(items)) = props.get("post_install_results") else {
        panic!("no post_install_results: {props:?}");
    };
    items
        .iter()
        .map(|item| {
            let PropertyValue::Object(o) = item else {
                panic!("entry is not an object")
            };
            let get = |k: &str| match o.get(k) {
                Some(PropertyValue::String(v)) => v.clone(),
                other => panic!("{k}: {other:?}"),
            };
            (get("function"), get("status"))
        })
        .collect()
}

const TWO_HOOKS: &str =
    "post_install:\n  - { function: /lib/a, input: { store: main } }\n  - function: /lib/b\n";

#[tokio::test]
async fn hooks_run_in_order_as_system_and_are_recorded() {
    let env = setup().await;
    let result = install(&env, "main", "sync", TWO_HOOKS)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(*env.ran.lock().unwrap(), ["/lib/a", "/lib/b"]);
    let results = &result["post_install_results"];
    assert_eq!(results[0]["function"], "/lib/a");
    assert_eq!(results[0]["status"], "completed");
    assert_eq!(results[1]["status"], "completed");

    let ctx = env.contexts.lock().unwrap()[0].clone();
    assert_eq!(ctx.branch, "main");
    assert_eq!(
        ctx.metadata["input"],
        serde_json::json!({ "store": "main" })
    );
    let auth: AuthContext = serde_json::from_value(ctx.metadata[AUTH_CONTEXT_KEY].clone()).unwrap();
    assert_eq!(
        auth.agent.as_deref(),
        Some("job:package-post-install:hooks")
    );
    // Apart from the agent marker, exactly the system context.
    let expected = AuthContext::system().with_agent("job:package-post-install:hooks");
    assert_eq!(
        serde_json::to_value(&auth).unwrap(),
        serde_json::to_value(&expected).unwrap(),
        "hooks run with the system context"
    );

    let props = package_props(&env).await;
    assert_eq!(props.get("installed"), Some(&PropertyValue::Boolean(true)));
    assert_eq!(
        statuses(&props),
        [
            ("/lib/a".to_string(), "completed".to_string()),
            ("/lib/b".to_string(), "completed".to_string())
        ]
    );
}

#[tokio::test]
async fn first_failure_stops_the_rest_and_fails_the_job_but_keeps_the_install() {
    let env = setup().await;
    let err = install(
        &env,
        "main",
        "sync",
        "post_install:\n  - function: /lib/a\n  - function: /lib/fail\n  - function: /lib/b\n",
    )
    .await
    .unwrap_err();
    // Permanent: the worker must not re-run the whole install on its
    // retry budget (see `worker::retry_allowed`).
    assert!(err.is_permanent(), "{err}");
    let err = err.to_string();

    assert!(
        err.starts_with("installed; post_install /lib/fail failed: "),
        "{err}"
    );
    assert!(err.contains("boom"), "{err}");
    assert_eq!(*env.ran.lock().unwrap(), ["/lib/a", "/lib/fail"]);

    let props = package_props(&env).await;
    assert_eq!(props.get("installed"), Some(&PropertyValue::Boolean(true)));
    assert_eq!(props.get("status"), Some(&s("failed")));
    assert_eq!(
        statuses(&props)
            .into_iter()
            .map(|(_, st)| st)
            .collect::<Vec<_>>(),
        ["completed", "failed", "skipped"]
    );
}

#[tokio::test]
async fn skip_mode_runs_the_hooks_on_every_install() {
    let env = setup().await;
    install(&env, "main", "skip", TWO_HOOKS).await.unwrap();
    install(&env, "main", "skip", TWO_HOOKS).await.unwrap();
    assert_eq!(
        *env.ran.lock().unwrap(),
        ["/lib/a", "/lib/b", "/lib/a", "/lib/b"]
    );
}

/// A node must never answer "the hooks ran" with the PREVIOUS install's
/// results: the CLI, when it cannot ask the job, waits for results to appear
/// after `installed`. Finalizing an install clears them; this install's hooks
/// record their own.
#[tokio::test]
async fn an_install_does_not_keep_the_previous_installs_hook_results() {
    let env = setup().await;
    install(&env, "main", "sync", TWO_HOOKS).await.unwrap();
    assert_eq!(statuses(&package_props(&env).await).len(), 2);

    install(&env, "main", "sync", "").await.unwrap();
    let props = package_props(&env).await;
    assert_eq!(props.get("status"), Some(&s("installed")));
    assert!(
        !props.contains_key("post_install_results"),
        "stale results survived a hook-less install: {:?}",
        props.get("post_install_results")
    );

    install(&env, "main", "sync", TWO_HOOKS).await.unwrap();
    assert_eq!(statuses(&package_props(&env).await).len(), 2);
}

/// A hook's own report survives: its `warnings` and a bounded rendering of
/// its return value are stored with its result row, which also names the
/// install job that ran it.
#[tokio::test]
async fn a_hooks_warnings_and_output_are_kept_bounded() {
    let env = setup().await;
    let result = install(
        &env,
        "main",
        "sync",
        "post_install:\n  - function: /lib/warn\n",
    )
    .await
    .unwrap()
    .unwrap();
    let row = &result["post_install_results"][0];
    assert_eq!(row["status"], "completed");
    let warnings = row["warnings"].as_array().expect("warnings");
    assert_eq!(warnings[0], "self-registration closed");
    assert_eq!(warnings[1], "audiences:/speakers deferred");
    assert!(
        warnings[2].as_str().unwrap().chars().count() <= 301,
        "clamped"
    );
    assert!(row["output"].as_str().unwrap().contains("\"seeded\":3"));

    let props = package_props(&env).await;
    let Some(PropertyValue::Array(items)) = props.get("post_install_results") else {
        panic!("no results on the node");
    };
    let PropertyValue::Object(stored) = &items[0] else {
        panic!("row is not an object");
    };
    assert!(matches!(stored.get("warnings"), Some(PropertyValue::Array(w)) if w.len() == 3));
    assert!(matches!(
        stored.get("output"),
        Some(PropertyValue::String(_))
    ));
    assert!(
        matches!(stored.get("job_id"), Some(PropertyValue::String(j)) if !j.is_empty()),
        "each row names the install job that produced it"
    );
}

#[test]
fn hook_output_summary_is_bounded() {
    use super::post_install::summarize_output;
    assert_eq!(summarize_output(None), (Vec::new(), None));
    assert_eq!(
        summarize_output(Some(&serde_json::Value::Null)),
        (Vec::new(), None)
    );
    let many: Vec<String> = (0..50).map(|i| format!("w{i}")).collect();
    let (w, out) = summarize_output(Some(&serde_json::json!({
        "warnings": many,
        "blob": "x".repeat(5_000)
    })));
    assert_eq!(w.len(), 20);
    assert!(out.unwrap().chars().count() <= 1_001);
    // A return value without warnings still keeps its output.
    let (w, out) = summarize_output(Some(&serde_json::json!("done")));
    assert!(w.is_empty());
    assert_eq!(out.as_deref(), Some("\"done\""));
}

/// The previous run's results are gone the moment a new run starts, not only
/// at finalize: a reader polling during the install never sees the new state
/// next to the old results.
#[tokio::test]
async fn a_new_run_clears_the_previous_results_at_start() {
    let env = setup().await;
    install(&env, "main", "sync", TWO_HOOKS).await.unwrap();
    assert!(package_props(&env)
        .await
        .contains_key("post_install_results"));

    let handler =
        PackageInstallHandler::new(env.storage.clone(), env.storage.job_registry().clone());
    let context = JobContext {
        tenant_id: TENANT.into(),
        repo_id: REPO.into(),
        branch: "main".into(),
        workspace_id: "packages".into(),
        revision: raisin_hlc::HLC::now(),
        metadata: HashMap::new(),
    };
    handler
        .set_package_status(&context, PKG_ID, "installing", None)
        .await;
    let props = package_props(&env).await;
    assert_eq!(props.get("status"), Some(&s("installing")));
    assert!(
        !props.contains_key("post_install_results"),
        "the old run's results must not sit next to the new run's status"
    );
}

/// `maravilla package deploy` installs on `main` (hooks run) and then the
/// schema on `publish`. The packages workspace pins `main`, so both installs
/// update ONE package node: the publish run must leave main's hook results
/// alone (it neither clears them at start or finalize, nor overwrites them
/// with its `skipped` rows).
#[tokio::test]
async fn a_publish_install_keeps_mains_hook_results() {
    let env = setup().await;
    install(&env, "main", "sync", TWO_HOOKS).await.unwrap();
    let before = statuses(&package_props(&env).await);
    assert_eq!(before.len(), 2);

    for post_install in ["", TWO_HOOKS] {
        install(&env, "feature", "sync", post_install)
            .await
            .unwrap();
        let props = package_props(&env).await;
        assert_eq!(
            statuses(&props),
            before,
            "a non-main install changed main's results (manifest hooks: {post_install:?})"
        );
    }
}

#[tokio::test]
async fn a_branch_other_than_main_runs_no_hooks() {
    let env = setup().await;
    let result = install(&env, "feature", "sync", TWO_HOOKS)
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(env.ran.lock().unwrap().is_empty());
    assert_eq!(result["post_install_results"][0]["status"], "skipped");
}

#[tokio::test]
async fn a_missing_function_fails_with_a_clear_message() {
    let env = setup().await;
    let err = install(
        &env,
        "main",
        "sync",
        "post_install:\n  - function: /lib/nope\n",
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.starts_with("installed; post_install /lib/nope failed: function not found"),
        "{err}"
    );
    assert!(env.ran.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_invalid_hook_list_fails_before_anything_installs() {
    let env = setup().await;
    let err = install(
        &env,
        "main",
        "sync",
        "post_install:\n  - function: lib/relative\n",
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("absolute"), "{err}");
    let props = package_props(&env).await;
    assert_ne!(props.get("installed"), Some(&PropertyValue::Boolean(true)));
}

#[tokio::test]
async fn dry_run_lists_the_hooks_without_running_them() {
    let env = setup().await;
    let handler =
        PackageInstallHandler::new(env.storage.clone(), env.storage.job_registry().clone());
    let zip = package_zip("post_install:\n  - function: /lib/a\n  - function: /lib/nope\n");
    let result = handler
        .dry_run(TENANT, REPO, "main", &zip, super::types::InstallMode::Skip)
        .await
        .unwrap();
    let hooks: Vec<_> = result
        .logs
        .iter()
        .filter(|l| l.category == "post_install")
        .collect();
    assert_eq!(hooks.len(), 2);
    assert_eq!(hooks[0].path, "/lib/a");
    assert_eq!(hooks[0].action, "run");
    assert!(
        hooks[1].message.contains("NOT FOUND"),
        "{}",
        hooks[1].message
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(env.ran.lock().unwrap().is_empty());

    let feature = handler
        .dry_run(
            TENANT,
            REPO,
            "feature",
            &zip,
            super::types::InstallMode::Skip,
        )
        .await
        .unwrap();
    assert!(feature
        .logs
        .iter()
        .filter(|l| l.category == "post_install")
        .all(|l| l.action == "skip"));
}

async fn stored_package(env: &Env) -> Node {
    env.storage
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, "main", "packages"),
            PKG_ID,
            None,
        )
        .await
        .unwrap()
        .expect("package node")
}

/// A failed hook leaves `installed: true` with `status: failed`, and that is
/// exactly the state in which a skip-mode install request starts a job again
/// (`skip_install_is_noop`); once the hooks pass, skip mode is a no-op again.
#[tokio::test]
async fn a_failed_hook_makes_the_next_skip_install_run_again() {
    use raisin_core::package_registration::skip_install_is_noop;
    let env = setup().await;
    let failing = "post_install:\n  - function: /lib/fail\n";
    install(&env, "main", "sync", failing).await.unwrap_err();
    assert!(!skip_install_is_noop(&stored_package(&env).await));

    install(&env, "main", "skip", TWO_HOOKS).await.unwrap();
    assert!(skip_install_is_noop(&stored_package(&env).await));
}

/// The hooks survive an export rebuilt from the package node (no archive).
#[tokio::test]
async fn export_from_the_package_node_keeps_post_install() {
    let env = setup().await;
    let hooks = "post_install:\n  - { function: /lib/a, input: { store: main, raisin:ref: /x, qty: '10' } }\n  - function: /lib/b\n";
    install(&env, "main", "sync", hooks).await.unwrap();

    let node = stored_package(&env).await;
    let exporter = crate::jobs::handlers::package_export::PackageExportHandler::new(
        env.storage.clone(),
        env.storage.job_registry().clone(),
    );
    let zip = exporter.build_minimal_package(&node).unwrap();

    let handler =
        PackageInstallHandler::new(env.storage.clone(), env.storage.job_registry().clone());
    let exported = handler.extract_manifest(&zip).unwrap();
    let original: super::manifest::PackageManifest =
        serde_yaml::from_str(&format!("name: hooks\nversion: 1.0.0\n{hooks}")).unwrap();
    assert_eq!(exported.post_install, original.post_install);
    assert_eq!(exported.post_install[0].input["qty"], "10");

    // A version without hooks removes them from the node, and from the export.
    install(&env, "main", "sync", "").await.unwrap();
    let node = stored_package(&env).await;
    assert!(!node
        .properties
        .contains_key(raisin_packages::POST_INSTALL_PROPERTY));
    let zip = exporter.build_minimal_package(&node).unwrap();
    assert!(handler
        .extract_manifest(&zip)
        .unwrap()
        .post_install
        .is_empty());
}
