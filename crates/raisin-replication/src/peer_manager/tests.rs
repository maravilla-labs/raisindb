#[cfg(test)]
mod tests {
    use crate::config::{ConnectionConfig, PeerConfig, RetryConfig};
    use crate::peer_manager::{ConnectionState, PeerManager};

    #[test]
    fn test_peer_manager_creation() {
        let manager = PeerManager::new(
            "node1".to_string(),
            ConnectionConfig::default(),
            RetryConfig::default(),
        );
        assert_eq!(manager.cluster_node_id, "node1");
    }

    #[tokio::test]
    async fn test_add_peer() {
        let manager = PeerManager::new(
            "node1".to_string(),
            ConnectionConfig::default(),
            RetryConfig::default(),
        );

        let peer = PeerConfig::new("node2", "10.0.1.2");
        manager.add_peer(peer).await;

        let status = manager.get_peer_status("node2").await;
        assert!(status.is_some());
        assert_eq!(status.unwrap().state, ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_peer_status() {
        let manager = PeerManager::new(
            "node1".to_string(),
            ConnectionConfig::default(),
            RetryConfig::default(),
        );

        manager.add_peer(PeerConfig::new("node2", "10.0.1.2")).await;
        manager.add_peer(PeerConfig::new("node3", "10.0.1.3")).await;

        let statuses = manager.get_all_peer_status().await;
        assert_eq!(statuses.len(), 2);
    }
}
