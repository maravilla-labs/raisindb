// Process management for cluster nodes

use super::config::{write_configs_to_dir, ClusterConfig, NodeConfig};
use super::social_feed::SOCIAL_FEED_REPO;
use anyhow::{Context, Result};
use reqwest::Client;
use std::fs::OpenOptions;
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
        let start = std::time::Instant::now();
        for index in 0..self.config.nodes().len() {
            let remaining = timeout.saturating_sub(start.elapsed());
            self.wait_for_node_health(index, remaining).await?;
        }
        println!("All nodes are healthy");
        Ok(())
    }

    /// Dump last lines of a node's logs.
    ///
    /// Reads the paths this cluster actually spawned with, rather than
    /// recomputing them from the node id — see [`Self::log_file_name`] for why
    /// those are not the same thing.
    fn dump_node_logs(&self, index: usize) {
        let Some(logs) = self.node_logs.get(index) else {
            return;
        };
        let (node_id, stdout_path, stderr_path) =
            (&logs.node_id, &logs.stdout_path, &logs.stderr_path);

        println!("\n=== Last 50 lines of {} logs ===", node_id);

        if let Ok(content) = std::fs::read_to_string(stdout_path) {
            let lines: Vec<&str> = content.lines().collect();
            let start = lines.len().saturating_sub(50);
            println!("\n--- stdout ---");
            for line in &lines[start..] {
                println!("{}", line);
            }
        }

        if let Ok(content) = std::fs::read_to_string(stderr_path) {
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

    /// Kill the node at `index` and start it again on the same data directory.
    ///
    /// Killing is what makes a peer genuinely unreachable, and that matters for
    /// any test about *undelivered* operations. Merely stopping a process
    /// (`SIGSTOP`) leaves it owning its listening socket, so the kernel keeps
    /// accepting and buffering replication bytes on its behalf and delivers the
    /// backlog the instant it resumes — in receive order, which is send order. A
    /// killed peer's socket is closed, the sender's delivery genuinely fails, and
    /// the operation stays in the sender's oplog until it reconnects. That is the
    /// only way to stage a real out-of-order delivery from outside the process.
    ///
    /// The data directory is preserved, so the node comes back with everything it
    /// had. Its log files are truncated by the respawn — take any
    /// log offsets you care about *after* the restart.
    pub fn restart_node(&mut self, index: usize) -> Result<()> {
        self.kill_node(index)?;
        self.start_node_again(index)
    }

    /// Kill the node at `index` and leave it down.
    ///
    /// Its data directory and log files survive; [`Self::start_node_again`] brings
    /// it back. See [`Self::restart_node`] for why a test wanting a peer to be
    /// unreachable must kill it rather than stop it.
    pub fn kill_node(&mut self, index: usize) -> Result<()> {
        let node_id = self
            .config
            .nodes
            .get(index)
            .map(|n| n.node_id.clone())
            .with_context(|| format!("no node at index {index}"))?;

        if let Some(child) = self.processes.get_mut(index) {
            let _ = child.kill();
            let _ = child.wait();
        }
        println!("  [restart] {node_id} killed");
        Ok(())
    }

    /// Start a node previously stopped by [`Self::kill_node`].
    ///
    /// The respawn truncates that node's log files, so take any log offsets you
    /// care about *after* calling this.
    pub fn start_node_again(&mut self, index: usize) -> Result<()> {
        let node = self
            .config
            .nodes
            .get(index)
            .cloned()
            .with_context(|| format!("no node at index {index}"))?;

        let binary_path = Self::find_binary()?;
        let config_path = self
            .config_dir
            .path()
            .join(format!("{}.toml", node.node_id));
        let (process, logs) = Self::spawn_node(&binary_path, &config_path, &node)
            .with_context(|| format!("failed to respawn {}", node.node_id))?;

        self.processes[index] = process;
        self.node_logs[index] = logs;
        println!("  [restart] {} started again", node.node_id);
        Ok(())
    }

    /// Wait for ONE node to answer its liveness route.
    ///
    /// `wait_for_health` polls every node and so cannot be used while any node is
    /// deliberately down or frozen; it delegates here per node.
    ///
    /// `/management/health` no longer exists — it moved to
    /// `/management/admin/health`, which is behind operator auth. `/health` is the
    /// unauthenticated liveness route (`state.rs`), and it is what
    /// `helpers::multi_node` has always used. A 401 counts as healthy: it means
    /// the process is up and serving, which is all a liveness check asks.
    pub async fn wait_for_node_health(&self, index: usize, timeout: Duration) -> Result<()> {
        let node = self
            .config
            .nodes
            .get(index)
            .with_context(|| format!("no node at index {index}"))?;
        let health_url = format!("{}/health", node.base_url());
        let client = Client::new();
        let start = std::time::Instant::now();

        loop {
            // 401 counts as healthy: the process is up and serving, which is all a
            // liveness check asks.
            if let Ok(resp) = client.get(&health_url).send().await {
                if resp.status().is_success() || resp.status().as_u16() == 401 {
                    println!("  {} is healthy again", node.node_id);
                    return Ok(());
                }
            }
            if start.elapsed() > timeout {
                let node_id = node.node_id.clone();
                self.dump_node_logs(index);
                anyhow::bail!(
                    "Node {} did not become healthy within {:?}",
                    node_id,
                    timeout
                );
            }
            sleep(Duration::from_millis(500)).await;
        }
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

    /// Log file name for one node, qualified by its HTTP port.
    ///
    /// The port is what makes this per-CLUSTER rather than per-node-id. Ports come
    /// from `ports::unique_ports`, so two suites running at once get different
    /// files; naming them `node1-stdout.log` (as this did) meant every cluster in
    /// the process wrote to the SAME three files, truncating each other on spawn.
    /// That is not only a debugging annoyance: `spatial_config_replication_test`
    /// reads these logs as evidence, and a concurrent run made it dump another
    /// cluster's node3 while reporting on its own.
    fn log_file_name(node: &NodeConfig, stream: &str) -> String {
        format!("{}-{}-{}.log", node.node_id, node.http_port, stream)
    }

    /// Spawn a single node process
    fn spawn_node(
        binary_path: &Path,
        config_path: &Path,
        node: &NodeConfig,
    ) -> Result<(Child, NodeLogs)> {
        // Create log files for this node
        let log_dir = std::env::temp_dir();
        let stdout_path = log_dir.join(Self::log_file_name(node, "stdout"));
        let stderr_path = log_dir.join(Self::log_file_name(node, "stderr"));

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
            // Without this the server exits(1) during startup — outside dev-mode it
            // demands JWT_SECRET, RAISINDB_SIGNING_SECRET and RAISIN_MASTER_KEY from
            // the ENVIRONMENT (main.rs), which no generated config file can supply.
            // Every cluster test in this suite was failing its health check for this
            // reason, with only an ERROR line in the node log to say why.
            // `helpers::multi_node`, which the single-node suites use, has always
            // passed it.
            .arg("--dev-mode")
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
