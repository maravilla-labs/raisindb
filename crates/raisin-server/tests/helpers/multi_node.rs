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
}

impl ServerConfig {
    /// Create a new server config with defaults
    pub fn new(port: u16) -> Self {
        let temp_dir = TempDir::new().unwrap();
        Self {
            port,
            data_dir: temp_dir.path().to_path_buf(),
            initial_admin_password: "admin123!@#".to_string(),
            cluster_node_id: None,
            replication_port: None,
        }
    }

    /// Set cluster configuration for replication
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
    _temp_dir: Option<TempDir>, // Keep temp dir alive
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

        // Create temp dir if using default config
        let temp_dir = if config.data_dir.to_str().unwrap().starts_with("/tmp") {
            Some(TempDir::new().unwrap())
        } else {
            None
        };

        // Start server process
        let mut cmd = Command::new(&binary_path);
        cmd.current_dir(&workspace_root)
            .env("RUST_LOG", "info")
            .arg("--port")
            .arg(config.port.to_string())
            .arg("--data-dir")
            .arg(&config.data_dir)
            .arg("--initial-admin-password")
            .arg(&config.initial_admin_password)
            .arg("--bind-address")
            .arg("127.0.0.1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

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
            _temp_dir: temp_dir,
        };

        // Wait for server to be ready
        if let Err(e) = handle.wait_for_ready(Duration::from_secs(30)).await {
            handle.kill();
            return Err(e);
        }

        Ok(handle)
    }

    /// Wait for server to be ready (health check)
    pub async fn wait_for_ready(&self, timeout: Duration) -> Result<(), String> {
        let client = Client::new();
        let health_url = format!("{}/management/health", self.base_url);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > timeout {
                return Err(format!(
                    "Server on port {} did not become ready within {:?}",
                    self.config.port, timeout
                ));
            }

            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => {
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
