// Test helpers for multi-node server testing

use reqwest::{Client, StatusCode};
use serde_json::json;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

/// Configuration for a test server instance
pub struct ServerConfig {
    pub port: u16,
    pub data_dir: PathBuf,
    pub initial_admin_password: String,
    pub cluster_node_id: Option<String>,
    pub replication_port: Option<u16>,
    /// When set, the PostgreSQL wire protocol listener is enabled on this port.
    ///
    /// `[pgwire] enabled` defaults to `false` (see `config.rs`), so a test that
    /// wants to speak to the server with a postgres client must ask for it — the
    /// port is also written into the TOML rather than passed as a flag so the
    /// merge order in `startup/cli.rs` is exercised the same way production is.
    pub pgwire_port: Option<u16>,
    /// `RUST_LOG` for the spawned server. Defaults to `info`.
    ///
    /// A test that asserts on a log line the server writes at `debug` needs to
    /// raise the level for that ONE target, and it must be per-server rather
    /// than an env var: these tests share a process, so a `set_var` would leak
    /// into every other server this binary starts.
    pub rust_log: String,
    _temp_dir: Option<TempDir>, // Keep alive so data_dir isn't deleted
}

impl ServerConfig {
    /// Create a new server config with defaults
    pub fn new(port: u16) -> Self {
        let temp_dir = TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        Self {
            port,
            data_dir,
            initial_admin_password: "Admin12345!@#".to_string(),
            cluster_node_id: None,
            replication_port: None,
            pgwire_port: None,
            rust_log: "info".to_string(),
            _temp_dir: Some(temp_dir),
        }
    }

    /// Enable the PostgreSQL wire protocol listener on `port`.
    #[allow(dead_code)]
    pub fn with_pgwire(mut self, port: u16) -> Self {
        self.pgwire_port = Some(port);
        self
    }

    /// Override the spawned server's `RUST_LOG`.
    #[allow(dead_code)]
    pub fn with_rust_log(mut self, rust_log: impl Into<String>) -> Self {
        self.rust_log = rust_log.into();
        self
    }

    /// Set cluster configuration for replication
    #[allow(dead_code)]
    pub fn with_cluster(mut self, node_id: String, replication_port: u16) -> Self {
        self.cluster_node_id = Some(node_id);
        self.replication_port = Some(replication_port);
        self
    }

    /// Get base URL for this server
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// Handle to a running server instance
pub struct ServerHandle {
    pub config: ServerConfig,
    pub process: Child,
    pub base_url: String,
}

impl ServerHandle {
    /// Start a new server instance
    pub async fn start(config: ServerConfig) -> Result<Self, String> {
        // Build server binary if needed
        let workspace_root = std::env::var("CARGO_MANIFEST_DIR")
            .map(|p| {
                PathBuf::from(p)
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .to_path_buf()
            })
            .unwrap_or_else(|_| PathBuf::from("../.."));

        let binary_path = workspace_root.join("target/debug/raisin-server");

        if !binary_path.exists() {
            println!("Building raisin-server...");
            let build_status = Command::new("cargo")
                .current_dir(&workspace_root)
                .args(&[
                    "build",
                    "--package",
                    "raisin-server",
                    "--features",
                    "storage-rocksdb",
                ])
                .status()
                .expect("Failed to build raisin-server");

            if !build_status.success() {
                return Err("Failed to build raisin-server".to_string());
            }
        }

        // Write TOML config (same approach as cluster tests)
        let config_path = config.data_dir.join("server.toml");
        let mut toml_content = format!(
            r#"[server]
port = {}
bind_address = "127.0.0.1"
data_dir = "{}"
initial_admin_password = "{}"
jwt_secret = "test-secret-key-for-integration-tests-only-32chars!"
"#,
            config.port,
            config.data_dir.display(),
            config.initial_admin_password,
        );
        if let Some(pgwire_port) = config.pgwire_port {
            toml_content.push_str(&format!(
                r#"
[pgwire]
enabled = true
bind_address = "127.0.0.1"
port = {pgwire_port}
max_connections = 16
"#
            ));
        }
        std::fs::write(&config_path, &toml_content)
            .map_err(|e| format!("Failed to write config: {}", e))?;

        // Start server process
        let mut cmd = Command::new(&binary_path);
        // Capture server output for debugging (RAISIN_TEST_SERVER_LOG=path
        // overrides; default lands in /tmp keyed by port)
        let log_path = std::env::var("RAISIN_TEST_SERVER_LOG")
            .unwrap_or_else(|_| format!("/tmp/raisin-test-server-{}.log", config.port));
        let log_file = std::fs::File::create(&log_path)
            .map_err(|e| format!("Failed to create server log file: {}", e))?;
        let log_file_err = log_file
            .try_clone()
            .map_err(|e| format!("Failed to clone log handle: {}", e))?;
        cmd.current_dir(&workspace_root)
            .env("RUST_LOG", &config.rust_log)
            .arg("--config")
            .arg(&config_path)
            .arg("--dev-mode")
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_file_err));

        if let Some(ref node_id) = config.cluster_node_id {
            cmd.arg("--cluster-node-id").arg(node_id);
        }

        if let Some(ref repl_port) = config.replication_port {
            cmd.arg("--replication-port").arg(repl_port.to_string());
        }

        let process = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn server process: {}", e))?;

        let base_url = config.base_url();

        let mut handle = Self {
            config,
            process,
            base_url: base_url.clone(),
        };

        // Wait for server to be ready
        if let Err(e) = handle.wait_for_ready(Duration::from_secs(60)).await {
            handle.kill();
            return Err(e);
        }

        Ok(handle)
    }

    /// Wait for server to be ready (health check)
    pub async fn wait_for_ready(&self, timeout: Duration) -> Result<(), String> {
        let client = Client::new();
        let health_url = format!("{}/health", self.base_url);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(format!(
                    "Server on port {} did not become ready within {:?}",
                    self.config.port, timeout
                ));
            }

            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 401 => {
                    // 200 = healthy, 401 = server is up but requires auth (also fine)
                    println!("✅ Server on port {} is ready", self.config.port);
                    return Ok(());
                }
                _ => {
                    sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    /// Kill the server process
    pub fn kill(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Authenticate and get JWT token
pub async fn authenticate(
    base_url: &str,
    tenant_id: &str,
    username: &str,
    password: &str,
) -> Result<String, String> {
    let client = Client::new();
    let auth_url = format!("{}/api/raisindb/sys/{}/auth", base_url, tenant_id);

    let response = client
        .post(&auth_url)
        .json(&json!({
            "username": username,
            "password": password,
            "interface": "console"
        }))
        .send()
        .await
        .map_err(|e| format!("Auth request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Authentication failed with status {}: {}",
            status, body
        ));
    }

    let auth_response: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse auth response: {}", e))?;

    auth_response["token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "No token in auth response".to_string())
}

/// Create a node via REST API
pub async fn create_node(
    base_url: &str,
    token: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    name: &str,
    node_type: &str,
    properties: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = Client::new();
    let url = format!(
        "{}/api/repository/{}/{}/head/{}/",
        base_url, repo, branch, workspace
    );

    let node_data = json!({
        "node": {
            "id": node_id,
            "name": name,
            "node_type": node_type,
            "properties": properties
        }
    });

    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&node_data)
        .send()
        .await
        .map_err(|e| format!("Create node request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Create node failed with status {}: {}",
            status, body
        ));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse create response: {}", e))
}

/// Get a node via REST API by path
pub async fn get_node_by_path(
    base_url: &str,
    token: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> Result<Option<serde_json::Value>, String> {
    let client = Client::new();
    let url = format!(
        "{}/api/repository/{}/{}/head/{}/{}",
        base_url, repo, branch, workspace, path
    );

    let response = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Get node request failed: {}", e))?;

    match response.status() {
        StatusCode::OK => {
            let node = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse node response: {}", e))?;
            Ok(Some(node))
        }
        StatusCode::NOT_FOUND => Ok(None),
        status => {
            let body = response.text().await.unwrap_or_default();
            Err(format!("Get node failed with status {}: {}", status, body))
        }
    }
}

/// Get a node via REST API by ID
pub async fn get_node_by_id(
    base_url: &str,
    token: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Result<Option<serde_json::Value>, String> {
    let client = Client::new();
    let url = format!(
        "{}/api/repository/{}/{}/head/{}/$ref/{}",
        base_url, repo, branch, workspace, node_id
    );

    let response = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("Get node by ID request failed: {}", e))?;

    match response.status() {
        StatusCode::OK => {
            let node = response
                .json()
                .await
                .map_err(|e| format!("Failed to parse node response: {}", e))?;
            Ok(Some(node))
        }
        StatusCode::NOT_FOUND => Ok(None),
        status => {
            let body = response.text().await.unwrap_or_default();
            Err(format!(
                "Get node by ID failed with status {}: {}",
                status, body
            ))
        }
    }
}

/// Wait for a node to appear (for replication testing)
pub async fn wait_for_node(
    base_url: &str,
    token: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout {
            return Err(format!(
                "Node {} did not appear within {:?}",
                node_id, timeout
            ));
        }

        if let Ok(Some(node)) =
            get_node_by_id(base_url, token, repo, branch, workspace, node_id).await
        {
            return Ok(node);
        }

        sleep(Duration::from_millis(100)).await;
    }
}
