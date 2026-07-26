// Configuration utilities for cluster node setup

use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Configuration for a single node in the cluster
#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub node_id: String,
    pub http_port: u16,
    pub replication_port: u16,
    pub data_dir: PathBuf,
    pub bind_address: String,
    pub initial_admin_password: String,
}

impl NodeConfig {
    /// Create a new node configuration
    pub fn new(node_id: String, http_port: u16, replication_port: u16, data_dir: PathBuf) -> Self {
        Self {
            node_id,
            http_port,
            replication_port,
            data_dir,
            bind_address: "127.0.0.1".to_string(),
            initial_admin_password: "Admin123!@#$".to_string(), // 12 chars: upper, lower, digit, special
        }
    }

    /// Get the base URL for this node's HTTP API
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.bind_address, self.http_port)
    }

    /// Get the replication address for this node
    pub fn replication_address(&self) -> String {
        format!("{}:{}", self.bind_address, self.replication_port)
    }
}

/// Configuration for an N-node cluster
#[derive(Debug, Clone)]
pub struct ClusterConfig {
    pub nodes: Vec<NodeConfig>,
}

impl ClusterConfig {
    /// Create a new cluster configuration with the given ports
    ///
    /// # Arguments
    /// * `ports` - Array of ports: [http1, repl1, http2, repl2, ...] (2 ports per node)
    pub fn new_with_ports(ports: &[u16]) -> anyhow::Result<Self> {
        if ports.len() % 2 != 0 {
            anyhow::bail!(
                "Expected even number of ports (2 per node), got {}",
                ports.len()
            );
        }

        if ports.len() < 4 {
            anyhow::bail!(
                "Expected at least 4 ports (2 nodes minimum), got {}",
                ports.len()
            );
        }

        let node_count = ports.len() / 2;
        let mut nodes = Vec::new();

        // Create nodes dynamically
        for i in 0..node_count {
            let node_id = format!("node{}", i + 1);
            let http_port = ports[i * 2];
            let repl_port = ports[i * 2 + 1];
            let data_dir = TempDir::new()?.into_path();

            nodes.push(NodeConfig::new(node_id, http_port, repl_port, data_dir));
        }

        Ok(Self { nodes })
    }

    /// Get all nodes as a Vec
    pub fn nodes(&self) -> Vec<&NodeConfig> {
        self.nodes.iter().collect()
    }

    /// Get base URLs for all nodes
    pub fn base_urls(&self) -> Vec<String> {
        self.nodes.iter().map(|n| n.base_url()).collect()
    }

    /// Get the number of nodes in this cluster
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// Generate a TOML configuration file for a single node
///
/// # Arguments
/// * `config` - Configuration for this node
/// * `peers` - Configurations for peer nodes
///
/// # Returns
/// TOML configuration as a string
pub fn generate_toml_config(config: &NodeConfig, peers: &[&NodeConfig]) -> String {
    let mut toml = format!(
        r#"[server]
port = {}
bind_address = "{}"
data_dir = "{}"
initial_admin_password = "{}"

[replication]
enabled = true
node_id = "{}"
port = {}
bind_address = "{}"

"#,
        config.http_port,
        config.bind_address,
        config.data_dir.display(),
        config.initial_admin_password,
        config.node_id,
        config.replication_port,
        config.bind_address,
    );

    // Add peers
    toml.push_str("[[replication.peers]]\n");
    for peer in peers {
        toml.push_str(&format!(
            r#"peer_id = "{}"
address = "{}"
port = {}

[[replication.peers]]
"#,
            peer.node_id, peer.bind_address, peer.replication_port
        ));
    }

    // Remove the last empty peer section
    if !peers.is_empty() {
        toml.truncate(toml.len() - "[[replication.peers]]\n".len());
    }

    // Add monitoring config (disabled for tests)
    toml.push_str(
        r#"
[monitoring]
enabled = false
interval_secs = 30
"#,
    );

    toml
}

/// Write TOML configuration files for all nodes in the cluster
///
/// # Arguments
/// * `cluster` - Cluster configuration
/// * `dir` - Directory to write config files to
///
/// # Returns
/// Paths to the generated config files for all nodes
pub fn write_configs_to_dir(cluster: &ClusterConfig, dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;

    let mut config_paths = Vec::new();

    // For each node, create a config with all other nodes as peers
    for (i, node) in cluster.nodes.iter().enumerate() {
        // Collect all other nodes as peers (full mesh topology)
        let peers: Vec<&NodeConfig> = cluster
            .nodes
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, n)| n)
            .collect();

        let config_path = dir.join(format!("{}.toml", node.node_id));
        let config_content = generate_toml_config(node, &peers);
        std::fs::write(&config_path, config_content)?;
        config_paths.push(config_path);
    }

    Ok(config_paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_config_urls() {
        let config = NodeConfig::new(
            "test_node".to_string(),
            8080,
            9000,
            PathBuf::from("/tmp/test"),
        );

        assert_eq!(config.base_url(), "http://127.0.0.1:8080");
        assert_eq!(config.replication_address(), "127.0.0.1:9000");
    }

    #[test]
    fn test_cluster_config_creation() {
        let ports = vec![8081, 9001, 8082, 9002, 8083, 9003];
        let cluster = ClusterConfig::new_with_ports(&ports).unwrap();

        assert_eq!(cluster.nodes.len(), 3);
        assert_eq!(cluster.nodes[0].http_port, 8081);
        assert_eq!(cluster.nodes[0].replication_port, 9001);
        assert_eq!(cluster.nodes[1].http_port, 8082);
        assert_eq!(cluster.nodes[1].replication_port, 9002);
        assert_eq!(cluster.nodes[2].http_port, 8083);
        assert_eq!(cluster.nodes[2].replication_port, 9003);
    }

    #[test]
    fn test_generate_toml_config() {
        let node1 = NodeConfig::new("node1".to_string(), 8081, 9001, PathBuf::from("/tmp/node1"));
        let node2 = NodeConfig::new("node2".to_string(), 8082, 9002, PathBuf::from("/tmp/node2"));

        let toml = generate_toml_config(&node1, &[&node2]);

        assert!(toml.contains("node_id = \"node1\""));
        assert!(toml.contains("port = 8081"));
        assert!(toml.contains("port = 9001"));
        assert!(toml.contains("peer_id = \"node2\""));
        assert!(toml.contains("enabled = true"));
    }
}
