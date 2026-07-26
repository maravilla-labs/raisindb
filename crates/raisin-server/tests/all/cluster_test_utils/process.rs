// Process management for cluster nodes

use super::config::{write_configs_to_dir, ClusterConfig, NodeConfig};
use super::social_feed::SOCIAL_FEED_REPO;
use anyhow::{Context, Result};
use reqwest::Client;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

/// Manages a 3-node cluster process group
pub struct ClusterProcess {
    pub config: ClusterConfig,
    pub processes: Vec<Child>,
    pub config_dir: TempDir,
    node_logs: Vec<NodeLogs>,
    preserve_data: bool,
}

/// Captures log file locations for a node process
#[derive(Debug, Clone)]
pub struct NodeLogs {
    pub node_id: String,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
}

impl ClusterProcess {
    /// Start a 3-node cluster with the given configuration
    ///
    /// This will:
    /// 1. Generate TOML config files for each node
    /// 2. Spawn raisin-server processes
    /// 3. Wait for health checks to pass
    pub async fn start(config: ClusterConfig) -> Result<Self> {
        // Create temp directory for config files
        let config_dir = TempDir::new().context("Failed to create temp config directory")?;

        // Write TOML config files
        let config_paths = write_configs_to_dir(&config, config_dir.path())
            .context("Failed to write config files")?;

        // Find binary path
        let binary_path = Self::find_binary()?;

        // Start all three processes
        let mut processes = Vec::new();
        let mut node_logs = Vec::new();

        for (idx, (node, config_path)) in config.nodes().iter().zip(config_paths.iter()).enumerate()
        {
            println!(
                "Starting {} (HTTP: {}, Replication: {})",
                node.node_id, node.http_port, node.replication_port
            );

            let (process, logs) = Self::spawn_node(&binary_path, config_path, node)
                .with_context(|| format!("Failed to spawn {}", node.node_id))?;

            processes.push(process);
            node_logs.push(logs);

            // Small delay between starting nodes (except after the last node)
            if idx < config.nodes().len() - 1 {
                sleep(Duration::from_millis(500)).await;
            }
        }

        let cluster = Self {
            config,
            processes,
            config_dir,
            node_logs,
            preserve_data: false,
        };

        Ok(cluster)
    }

    /// Wait for all nodes to become healthy
    pub async fn wait_for_health(&self, timeout: Duration) -> Result<()> {
        let client = Client::new();
        let start = std::time::Instant::now();

        for (idx, node) in self.config.nodes().iter().enumerate() {
            let health_url = format!("{}/management/health", node.base_url());

            loop {
                if start.elapsed() > timeout {
                    // Dump logs for failed node
                    Self::dump_node_logs(&node.node_id);

                    anyhow::bail!(
                        "Node {} did not become healthy within {:?}\nCheck logs at /tmp/{}-{{stdout,stderr}}.log",
                        node.node_id,
                        timeout,
                        node.node_id
                    );
                }

                match client.get(&health_url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        println!("  {} is healthy", node.node_id);
                        break;
                    }
                    Ok(resp) => {
                        eprintln!(
                            "  {} health check returned status: {}",
                            node.node_id,
                            resp.status()
                        );
                        sleep(Duration::from_millis(500)).await;
                    }
                    Err(e) => {
                        if start.elapsed().as_secs() % 5 == 0
                            && start.elapsed().as_millis() % 500 < 100
                        {
                            eprintln!("  {} not ready yet: {}", node.node_id, e);
                        }
                        sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }

        println!("All nodes are healthy");
        Ok(())
    }

    /// Dump last lines of a node's logs
    fn dump_node_logs(node_id: &str) {
        let log_dir = std::env::temp_dir();
        let stdout_path = log_dir.join(format!("{}-stdout.log", node_id));
        let stderr_path = log_dir.join(format!("{}-stderr.log", node_id));

        println!("\n=== Last 50 lines of {} logs ===", node_id);

        if let Ok(content) = std::fs::read_to_string(&stdout_path) {
            let lines: Vec<&str> = content.lines().collect();
            let start = lines.len().saturating_sub(50);
            println!("\n--- stdout ---");
            for line in &lines[start..] {
                println!("{}", line);
            }
        }

        if let Ok(content) = std::fs::read_to_string(&stderr_path) {
            let lines: Vec<&str> = content.lines().collect();
            let start = lines.len().saturating_sub(50);
            println!("\n--- stderr ---");
            for line in &lines[start..] {
                println!("{}", line);
            }
        }
        println!("=== End of {} logs ===\n", node_id);
    }

    /// Mark data directories to be preserved after test completion
    pub fn preserve_on_failure(&mut self) {
        self.preserve_data = true;
        println!("\nPreserving data directories for debugging:");
        for node in self.config.nodes() {
            println!("  {}: {}", node.node_id, node.data_dir.display());
        }
    }

    /// Clean up data directories (called on successful test completion)
    pub fn cleanup_on_success(&self) {
        if !self.preserve_data {
            for node in self.config.nodes() {
                if node.data_dir.exists() {
                    if let Err(e) = std::fs::remove_dir_all(&node.data_dir) {
                        eprintln!(
                            "Warning: Failed to clean up {}: {}",
                            node.data_dir.display(),
                            e
                        );
                    }
                }
            }
        }
    }

    /// Gracefully shutdown all nodes
    pub fn shutdown(&mut self) {
        for (process, node) in self.processes.iter_mut().zip(self.config.nodes()) {
            println!("Shutting down {}...", node.node_id);
            let _ = process.kill();
            let _ = process.wait();
        }
    }

    /// Return the log file paths for each node
    pub fn log_paths(&self) -> &[NodeLogs] {
        &self.node_logs
    }

    /// Find the raisin-server binary (release or debug)
    fn find_binary() -> Result<PathBuf> {
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

        // Prefer the fresh debug binary when running tests
        let debug_path = workspace_root.join("target/debug/raisin-server");
        if debug_path.exists() {
            println!("Using debug binary: {}", debug_path.display());
            return Ok(debug_path);
        }

        let release_path = workspace_root.join("target/release/raisin-server");
        if release_path.exists() {
            println!("Using release binary: {}", release_path.display());
            return Ok(release_path);
        }

        // Try to build it
        println!("Binary not found, building raisin-server...");
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
            .context("Failed to execute cargo build")?;

        if !build_status.success() {
            anyhow::bail!("Failed to build raisin-server");
        }

        let debug_path = workspace_root.join("target/debug/raisin-server");
        if debug_path.exists() {
            Ok(debug_path)
        } else {
            anyhow::bail!("Binary not found after build")
        }
    }

    /// Spawn a single node process
    fn spawn_node(
        binary_path: &Path,
        config_path: &Path,
        node: &NodeConfig,
    ) -> Result<(Child, NodeLogs)> {
        // Create log files for this node
        let log_dir = std::env::temp_dir();
        let stdout_path = log_dir.join(format!("{}-stdout.log", node.node_id));
        let stderr_path = log_dir.join(format!("{}-stderr.log", node.node_id));

        let stdout_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&stdout_path)
            .with_context(|| format!("Failed to create stdout log file for {}", node.node_id))?;

        let stderr_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&stderr_path)
            .with_context(|| format!("Failed to create stderr log file for {}", node.node_id))?;

        println!(
            "  {} logs: stdout={:?}, stderr={:?}",
            node.node_id, stdout_path, stderr_path
        );

        let child = Command::new(binary_path)
            .env(
                "RUST_LOG",
                "info,raisin_replication=debug,raisin_server=debug,raisin_http=debug,node_service::get_by_path=debug,node_service::workspace_delta=debug,rocksb::nodes::revision_lookup=debug,rocksb::nodetype::lookup=debug",
            )
            .env(
                "RAISIN_CLUSTER_SYNC_EXTRA_REPOS",
                format!("default:{}", SOCIAL_FEED_REPO),
            )
            .arg("--config")
            .arg(config_path)
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .spawn()
            .with_context(|| format!("Failed to spawn process for {}", node.node_id))?;

        let logs = NodeLogs {
            node_id: node.node_id.clone(),
            stdout_path,
            stderr_path,
        };

        Ok((child, logs))
    }
}

impl Drop for ClusterProcess {
    fn drop(&mut self) {
        self.shutdown();
        if !self.preserve_data {
            self.cleanup_on_success();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_binary() {
        // This test just verifies the function doesn't panic
        // It may trigger a build, which is acceptable in test context
        let result = ClusterProcess::find_binary();
        assert!(
            result.is_ok(),
            "Should find or build binary: {:?}",
            result.err()
        );
    }
}
