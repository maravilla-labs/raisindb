// Test fixture for cluster integration tests
use super::config::ClusterConfig;
use super::ports::unique_ports;
use super::process::ClusterProcess;
use super::rest_client::RestClient;
use super::social_feed::{
    add_follow_relationships, create_demo_users, create_initial_posts, init_social_feed_schema,
    SOCIAL_FEED_BRANCH, SOCIAL_FEED_NODE_TYPES, SOCIAL_FEED_REPO, SOCIAL_FEED_WORKSPACE,
};
use super::verification::{wait_for_replication, wait_for_replication_by_id};
use crate::cluster_test_utils::verification::wait_for_nodetype_replication;
use anyhow::{Context, Result};
use std::time::Duration;

/// Complete test fixture for a 3-node cluster with social feed schema
///
/// This fixture provides:
/// - 3 running RaisinDB nodes
/// - REST client configured for all nodes
/// - Authentication tokens for all nodes
/// - Social feed schema initialized
/// - Demo users and initial posts created
pub struct ClusterTestFixture {
    pub cluster: ClusterProcess,
    pub client: RestClient,
    pub tokens: Vec<String>,
    pub config: ClusterConfig,
    pub user_ids: Vec<String>,
    pub user_paths: Vec<String>,
    pub post_ids: Vec<String>,
    pub post_paths: Vec<String>,
}

impl ClusterTestFixture {
    /// Set up a complete test cluster with default 3 nodes
    pub async fn setup() -> Result<Self> {
        Self::setup_with_nodes(3).await
    }

    /// Set up a complete test cluster with specified number of nodes (2+)
    ///
    /// This performs the following steps:
    /// 1. Allocate unique ports for HTTP and replication
    /// 2. Start cluster with N nodes
    /// 3. Wait for all nodes to become healthy
    /// 4. Authenticate to all nodes
    /// 5. Initialize social feed schema on node1
    /// 6. Wait for schema to replicate
    /// 7. Create demo users and posts
    /// 8. Wait for initial data to replicate
    pub async fn setup_with_nodes(node_count: usize) -> Result<Self> {
        if node_count < 2 {
            anyhow::bail!("Node count must be at least 2, got {}", node_count);
        }

        println!(
            "\n=== Setting up {}-node cluster test fixture ===\n",
            node_count
        );

        // Ensure all nodes sync the social feed repository even before it exists locally
        std::env::set_var(
            "RAISIN_CLUSTER_SYNC_EXTRA_REPOS",
            format!("default:{}", SOCIAL_FEED_REPO),
        );

        // Step 1: Allocate ports
        println!("Step 1: Allocating ports...");
        let port_count = node_count * 2; // 2 ports per node (HTTP + replication)
        let ports = unique_ports(port_count);

        // Print port allocations dynamically
        let http_ports: Vec<String> = (0..node_count).map(|i| ports[i * 2].to_string()).collect();
        let repl_ports: Vec<String> = (0..node_count)
            .map(|i| ports[i * 2 + 1].to_string())
            .collect();
        println!(
            "  Allocated ports: HTTP=[{}], Replication=[{}]",
            http_ports.join(", "),
            repl_ports.join(", ")
        );

        // Step 2: Create cluster config and start processes
        println!("\nStep 2: Starting cluster...");
        let config = ClusterConfig::new_with_ports(&ports)?;
        let cluster = ClusterProcess::start(config.clone())
            .await
            .context("Failed to start cluster")?;

        // Step 3: Wait for health checks
        println!("\nStep 3: Waiting for health checks...");
        cluster
            .wait_for_health(Duration::from_secs(30))
            .await
            .context("Cluster failed to become healthy")?;

        // Step 4: Create REST client and authenticate
        println!("\nStep 4: Authenticating to all nodes...");
        let client = RestClient::new(cluster.config.base_urls());

        // Wait for admin user to be created by the event handler
        // The AdminUserInitHandler listens for TenantCreated events and creates the admin user asynchronously
        println!("  Waiting for admin user to be initialized...");
        tokio::time::sleep(Duration::from_secs(2)).await;

        let mut tokens = Vec::new();
        for (idx, url) in client.base_urls.iter().enumerate() {
            // Retry authentication with exponential backoff (admin user might not be created yet)
            let mut attempts = 0;
            let max_attempts = 10;
            let mut delay = Duration::from_millis(500);

            let token = loop {
                attempts += 1;

                match client
                    .authenticate(url, "default", "admin", "Admin123!@#$")
                    .await
                {
                    Ok(token) => break token,
                    Err(e) if attempts < max_attempts => {
                        println!(
                            "  Authentication attempt {}/{} failed for node{}: {}. Retrying in {:?}...",
                            attempts, max_attempts, idx + 1, e, delay
                        );
                        tokio::time::sleep(delay).await;
                        delay = delay.saturating_mul(2); // Exponential backoff
                    }
                    Err(e) => {
                        return Err(e).with_context(|| {
                            format!(
                                "Failed to authenticate to node{} after {} attempts. \
                                Admin user may not have been created yet by the TenantCreated event handler.",
                                idx + 1, attempts
                            )
                        });
                    }
                }
            };

            tokens.push(token);
            println!(
                "  Authenticated to node{} after {} attempt(s)",
                idx + 1,
                attempts
            );
        }

        // Step 4.5: Wait for peer connectivity to be established
        println!("\nStep 4.5: Waiting for peer connectivity...");
        println!("  Allowing time for all nodes to establish peer connections...");
        // Give nodes time to complete peer connection establishment
        // This ensures the replication mesh is fully connected before we start creating data
        tokio::time::sleep(Duration::from_secs(5)).await;
        println!("  Peer connectivity wait complete");

        // Step 5: Initialize schema on node1
        println!("\nStep 5: Initializing social feed schema on node1...");
        init_social_feed_schema(&client, &client.base_urls[0], &tokens[0])
            .await
            .context("Failed to initialize schema")?;

        // Wait for schema replication - verify NodeTypes exist on all nodes
        println!("  Waiting for schema to replicate...");
        tokio::time::sleep(Duration::from_millis(250)).await;
        for node_type in SOCIAL_FEED_NODE_TYPES {
            wait_for_nodetype_replication(
                &client,
                &tokens,
                SOCIAL_FEED_REPO,
                SOCIAL_FEED_BRANCH,
                node_type,
                Duration::from_secs(45),
            )
            .await
            .with_context(|| format!("NodeType {} failed to replicate", node_type))?;
        }
        println!("  All NodeTypes replicated successfully");

        // Ensure repositories and workspaces exist on all nodes before creating data
        for idx in 1..client.base_urls.len() {
            let url = &client.base_urls[idx];
            let token = &tokens[idx];

            if let Err(e) = client.create_repository(url, token, SOCIAL_FEED_REPO).await {
                if !format!("{}", e).contains("already exists") {
                    return Err(
                        e.context(format!("Failed to create repository on node{}", idx + 1))
                    );
                }
            }

            if let Err(e) = client
                .create_workspace(url, token, SOCIAL_FEED_REPO, SOCIAL_FEED_WORKSPACE)
                .await
            {
                if !format!("{}", e).contains("already exists") {
                    return Err(e.context(format!("Failed to create workspace on node{}", idx + 1)));
                }
            }
        }

        // Step 6: Create demo users on node1
        println!("\nStep 6: Creating demo users on node1...");
        let user_records = create_demo_users(&client, &client.base_urls[0], &tokens[0])
            .await
            .context("Failed to create demo users")?;
        let user_ids: Vec<String> = user_records.iter().map(|(id, _)| id.clone()).collect();
        let user_paths: Vec<String> = user_records.iter().map(|(_, path)| path.clone()).collect();

        // Wait for user replication
        println!("  Checking user visibility on each node (before replication wait)...");
        for (user_id, user_path) in user_ids.iter().zip(user_paths.iter()) {
            log_node_presence_by_path(
                &client,
                &tokens,
                SOCIAL_FEED_REPO,
                SOCIAL_FEED_BRANCH,
                SOCIAL_FEED_WORKSPACE,
                &user_path,
                &format!("user {}", user_id),
            )
            .await;
        }
        println!("  Waiting for users to replicate...");
        for user_path in &user_paths {
            wait_for_replication(
                &client,
                &tokens,
                SOCIAL_FEED_REPO,
                SOCIAL_FEED_BRANCH,
                SOCIAL_FEED_WORKSPACE,
                &user_path,
                Duration::from_secs(40),
            )
            .await
            .with_context(|| format!("User {} failed to replicate", user_path))?;
        }
        println!("  All users replicated successfully");

        // Step 7: Create initial posts on node1
        println!("\nStep 7: Creating initial posts on node1...");
        let post_records =
            create_initial_posts(&client, &client.base_urls[0], &tokens[0], &user_records)
                .await
                .context("Failed to create initial posts")?;
        let post_ids: Vec<String> = post_records.iter().map(|(id, _)| id.clone()).collect();
        let post_paths: Vec<String> = post_records.iter().map(|(_, path)| path.clone()).collect();

        // Wait for post replication
        println!("  Checking post visibility on each node (before replication wait)...");
        for (post_id, post_path) in &post_records {
            log_node_presence_by_path(
                &client,
                &tokens,
                SOCIAL_FEED_REPO,
                SOCIAL_FEED_BRANCH,
                SOCIAL_FEED_WORKSPACE,
                post_path,
                &format!("post {}", post_id),
            )
            .await;
        }
        println!("  Waiting for posts to replicate...");
        for (post_id, post_path) in &post_records {
            wait_for_replication(
                &client,
                &tokens,
                SOCIAL_FEED_REPO,
                SOCIAL_FEED_BRANCH,
                SOCIAL_FEED_WORKSPACE,
                post_path,
                Duration::from_secs(40),
            )
            .await
            .with_context(|| format!("Post {} failed to replicate", post_id))?;
        }
        println!("  All posts replicated successfully");

        // Step 8: Add follow relationships
        println!("\nStep 8: Adding follow relationships...");
        add_follow_relationships(&client, &client.base_urls[0], &tokens[0], &user_paths)
            .await
            .context("Failed to add follow relationships")?;

        // Wait for relationships to replicate
        tokio::time::sleep(Duration::from_secs(2)).await;

        println!("\n=== Cluster test fixture ready ===\n");

        Ok(Self {
            cluster,
            client,
            tokens,
            config,
            user_ids,
            user_paths,
            post_ids,
            post_paths,
        })
    }

    /// Clean up resources after successful test
    pub fn teardown(mut self) {
        println!("\nCleaning up cluster...");
        self.cluster.shutdown();
        self.cluster.cleanup_on_success();
        println!("Cleanup complete");
    }

    /// Preserve data directories for debugging after test failure
    pub fn teardown_on_failure(mut self) {
        println!("\nTest failed - preserving cluster data for debugging");
        self.cluster.preserve_on_failure();
        self.cluster.shutdown();
    }

    /// Get the repository ID
    pub fn repo(&self) -> &str {
        SOCIAL_FEED_REPO
    }

    /// Get the workspace ID
    pub fn workspace(&self) -> &str {
        SOCIAL_FEED_WORKSPACE
    }

    /// Get the branch name
    pub fn branch(&self) -> &str {
        SOCIAL_FEED_BRANCH
    }
}

impl Drop for ClusterTestFixture {
    fn drop(&mut self) {
        // Ensure cleanup happens even if teardown wasn't called explicitly
        self.cluster.shutdown();
    }
}

async fn log_node_presence_by_path(
    client: &RestClient,
    tokens: &[String],
    repo: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
    label: &str,
) {
    println!("    Visibility check for {} (path='{}'):", label, node_path);
    for (idx, (url, token)) in client.base_urls.iter().zip(tokens.iter()).enumerate() {
        match client
            .get_node(url, token, repo, branch, workspace, node_path)
            .await
        {
            Ok(Some(node)) => {
                let name = node["name"].as_str().unwrap_or("no-name");
                let node_id = node["id"].as_str().unwrap_or("unknown-id");
                println!(
                    "      node{} -> FOUND (id='{}', name='{}')",
                    idx + 1,
                    node_id,
                    name,
                );
            }
            Ok(None) => println!("      node{} -> NOT FOUND (404)", idx + 1),
            Err(e) => println!(
                "      node{} -> ERROR while fetching {}: {}",
                idx + 1,
                label,
                e
            ),
        }
    }
}
