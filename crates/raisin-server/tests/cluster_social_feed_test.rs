// Comprehensive 3-node cluster integration tests for RaisinDB
//
// These tests verify:
// - CRUD operations across cluster nodes
// - Natural child ordering consistency (from fragmented index)
// - Replication correctness
// - REST and SQL API consistency

mod cluster_test_utils;

use cluster_test_utils::{
    create_comment, create_post, verify_child_order_via_rest, verify_child_order_via_sql,
    verify_comment_exists_on_all_nodes, verify_node_exists_on_all_nodes,
    verify_node_properties_match, verify_post_at_same_position,
    verify_relation_deleted_on_all_nodes, verify_relation_exists_on_all_nodes,
    verify_relations_match, wait_for_replication, ClusterTestFixture,
};
use serde_json::json;
use std::time::{Duration, Instant};

/// Test 1: Verify cluster initialization and basic setup
#[tokio::test]
#[ignore] // Run with --ignored flag
async fn test_cluster_initialization() {
    println!("\n=== Test: Cluster Initialization ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    // Verify all users exist on all nodes
    for user_path in &fixture.user_paths {
        verify_node_exists_on_all_nodes(
            &fixture.client,
            &fixture.tokens,
            fixture.repo(),
            fixture.branch(),
            fixture.workspace(),
            user_path,
        )
        .await
        .expect(&format!("User {} not on all nodes", user_path));
    }

    // Verify all posts exist on all nodes
    for post_path in &fixture.post_paths {
        verify_node_exists_on_all_nodes(
            &fixture.client,
            &fixture.tokens,
            fixture.repo(),
            fixture.branch(),
            fixture.workspace(),
            post_path,
        )
        .await
        .expect(&format!("Post {} not on all nodes", post_path));
    }

    println!("\n✅ Cluster initialization test passed\n");
    fixture.teardown();
}

/// Test 2: Add post on node1, verify natural order on all nodes
#[tokio::test]
#[ignore]
async fn test_add_post_node1_check_natural_order() {
    println!("\n=== Test: Add Post on Node1, Check Natural Order ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    // Get initial children count for alice
    let alice_path = &fixture.user_paths[0];
    let initial_children = fixture
        .client
        .list_children(
            &fixture.client.base_urls[0],
            &fixture.tokens[0],
            fixture.repo(),
            fixture.branch(),
            fixture.workspace(),
            alice_path,
        )
        .await
        .expect("Failed to list alice's children");

    println!("Alice has {} posts initially", initial_children.len());

    // Create a new post on node1
    println!("\nCreating new post on node1...");
    let new_post_id = "post_new_1";
    create_post(
        &fixture.client,
        &fixture.client.base_urls[0],
        &fixture.tokens[0],
        alice_path,
        new_post_id,
        "New Post from Node1",
        "This post was created on node1 to test ordering",
    )
    .await
    .expect("Failed to create post");

    // Wait for replication
    println!("Waiting for replication...");
    wait_for_replication(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        &format!("{}/{}", alice_path, new_post_id),
        Duration::from_secs(10),
    )
    .await
    .expect("Post did not replicate");

    // Verify child order is consistent across all nodes
    println!("\nVerifying child order consistency...");
    verify_child_order_via_rest(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        alice_path,
    )
    .await
    .expect("Child order verification failed");

    println!("\n✅ Natural order test (node1) passed\n");
    fixture.teardown();
}

/// Test 3: Add post on node2, verify natural order on all nodes
#[tokio::test]
#[ignore]
async fn test_add_post_node2_check_natural_order() {
    println!("\n=== Test: Add Post on Node2, Check Natural Order ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    let bob_path = &fixture.user_paths[1];

    // Create a new post on node2
    println!("Creating new post on node2...");
    let new_post_id = "post_new_2";
    create_post(
        &fixture.client,
        &fixture.client.base_urls[1], // node2
        &fixture.tokens[1],
        bob_path,
        new_post_id,
        "New Post from Node2",
        "This post was created on node2 to test ordering",
    )
    .await
    .expect("Failed to create post");

    // Wait for replication
    println!("Waiting for replication...");
    wait_for_replication(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        &format!("{}/{}", bob_path, new_post_id),
        Duration::from_secs(10),
    )
    .await
    .expect("Post did not replicate");

    // Verify child order is consistent across all nodes
    println!("\nVerifying child order consistency...");
    verify_child_order_via_rest(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        bob_path,
    )
    .await
    .expect("Child order verification failed");

    println!("\n✅ Natural order test (node2) passed\n");
    fixture.teardown();
}

/// Test 4: Add post on node3, verify natural order on all nodes
#[tokio::test]
#[ignore]
async fn test_add_post_node3_check_natural_order() {
    println!("\n=== Test: Add Post on Node3, Check Natural Order ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    let carol_path = &fixture.user_paths[2];

    // Create a new post on node3
    println!("Creating new post on node3...");
    let new_post_id = "post_new_3";
    create_post(
        &fixture.client,
        &fixture.client.base_urls[2], // node3
        &fixture.tokens[2],
        carol_path,
        new_post_id,
        "New Post from Node3",
        "This post was created on node3 to test ordering",
    )
    .await
    .expect("Failed to create post");

    // Wait for replication
    println!("Waiting for replication...");
    wait_for_replication(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        &format!("{}/{}", carol_path, new_post_id),
        Duration::from_secs(10),
    )
    .await
    .expect("Post did not replicate");

    // Verify child order is consistent across all nodes
    println!("\nVerifying child order consistency...");
    verify_child_order_via_rest(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        carol_path,
    )
    .await
    .expect("Child order verification failed");

    println!("\n✅ Natural order test (node3) passed\n");
    fixture.teardown();
}

/// Test 5: Update post created on different node
#[tokio::test]
#[ignore]
async fn test_update_post_across_nodes() {
    println!("\n=== Test: Update Post Across Nodes ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    let alice_path = &fixture.user_paths[0];
    let post_path = fixture.post_paths[0].clone();

    // Update post on node2 (post was created on node1)
    println!("Updating post on node2 (created on node1)...");
    let updated_data = json!({
        "properties": {
            "title": "Updated Title",
            "content": "This content was updated on node2",
            "likes_count": 42
        }
    });

    fixture
        .client
        .update_node(
            &fixture.client.base_urls[1], // node2
            &fixture.tokens[1],
            fixture.repo(),
            fixture.branch(),
            fixture.workspace(),
            &post_path,
            updated_data,
        )
        .await
        .expect("Failed to update post");

    // Wait for replication
    println!("Waiting for update to replicate...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify properties match on all nodes
    println!("Verifying properties match...");
    verify_node_properties_match(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        &post_path,
    )
    .await
    .expect("Properties don't match across nodes");

    println!("\n✅ Update post test passed\n");
    fixture.teardown();
}

/// Test 6: Verify relations replicate correctly
#[tokio::test]
#[ignore]
async fn test_relations_likes_via_rest() {
    println!("\n=== Test: Relations (Likes) via REST ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    let alice_path = &fixture.user_paths[0];
    let bob_path = &fixture.user_paths[1];
    let post_path = fixture.post_paths[0].clone();

    // Bob likes Alice's post (on node2)
    println!("Adding 'likes' relation on node2...");
    fixture
        .client
        .add_relation(
            &fixture.client.base_urls[1], // node2
            &fixture.tokens[1],
            fixture.repo(),
            fixture.branch(),
            fixture.workspace(),
            bob_path,
            &post_path,
            "likes",
        )
        .await
        .expect("Failed to add like relation");

    // Wait for replication
    println!("Waiting for relation to replicate...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify relation exists on all nodes
    println!("Verifying relation on all nodes...");
    verify_relations_match(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        bob_path,
        3, // 2 follows + 1 like
    )
    .await
    .expect("Relations don't match across nodes");

    println!("\n✅ Relations test passed\n");
    fixture.teardown();
}

/// Test 7b: Full lifecycle across nodes (create/update/delete/relations)
#[tokio::test]
#[ignore]
async fn test_cross_node_post_lifecycle() {
    println!("\n=== Test: Cross-Node Post Lifecycle ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    let repo = fixture.repo();
    let branch = fixture.branch();
    let workspace = fixture.workspace();

    let user_paths = fixture.user_paths.clone();

    // 1. Create a new post from each cluster node (one per user)
    println!("Creating cluster-scoped posts from every node...");
    let creation_specs = vec![
        (
            0,
            0,
            "cluster_post_node1",
            "Cluster Post from Node1",
            "Created on node1 to verify replication + ordering",
        ),
        (
            1,
            1,
            "cluster_post_node2",
            "Cluster Post from Node2",
            "Created on node2 to verify replication + ordering",
        ),
        (
            2,
            2,
            "cluster_post_node3",
            "Cluster Post from Node3",
            "Created on node3 to verify replication + ordering",
        ),
    ];

    let mut cluster_post_paths = Vec::new();

    for (node_idx, user_idx, post_id, title, content) in creation_specs {
        let author_path = &user_paths[user_idx];
        create_post(
            &fixture.client,
            &fixture.client.base_urls[node_idx],
            &fixture.tokens[node_idx],
            author_path,
            post_id,
            title,
            content,
        )
        .await
        .expect("Failed to create cluster-scoped post");

        let post_path = format!("{}/{}", author_path, post_id);
        wait_for_replication(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            &post_path,
            Duration::from_secs(15),
        )
        .await
        .expect("Cluster post failed to replicate");

        verify_node_exists_on_all_nodes(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            &post_path,
        )
        .await
        .expect("Cluster post missing on a node");

        cluster_post_paths.push(post_path);
    }

    // Ensure natural ordering is identical on every node for the affected users
    for user_path in &user_paths {
        verify_child_order_via_rest(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            user_path,
        )
        .await
        .expect("Child order mismatch after multi-node create");
    }

    // 2. Update each new post from a *different* node than the creator
    println!("Updating cluster posts from different nodes...");
    let update_specs = vec![
        (
            1usize,
            cluster_post_paths[0].clone(),
            json!({
                "properties": {
                    "title": "Node1 post updated from node2",
                    "content": "Updated on node2 to prove cross-node writes",
                    "likes_count": 11
                }
            }),
        ),
        (
            2usize,
            cluster_post_paths[1].clone(),
            json!({
                "properties": {
                    "title": "Node2 post updated from node3",
                    "content": "Updated on node3 to prove cross-node writes",
                    "likes_count": 22
                }
            }),
        ),
        (
            0usize,
            cluster_post_paths[2].clone(),
            json!({
                "properties": {
                    "title": "Node3 post updated from node1",
                    "content": "Updated on node1 to prove cross-node writes",
                    "likes_count": 33
                }
            }),
        ),
    ];

    for (node_idx, post_path, payload) in &update_specs {
        fixture
            .client
            .update_node(
                &fixture.client.base_urls[*node_idx],
                &fixture.tokens[*node_idx],
                repo,
                branch,
                workspace,
                post_path,
                payload.clone(),
            )
            .await
            .expect("Failed to update cluster post");

        tokio::time::sleep(Duration::from_secs(3)).await;

        verify_node_properties_match(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            post_path,
        )
        .await
        .expect("Post properties diverged after cross-node update");
    }

    // 3. Delete one of the posts from a third node and verify it disappears everywhere
    println!("Deleting node2's post from node3 and verifying removal...");
    let delete_target = cluster_post_paths[1].clone();
    fixture
        .client
        .delete_node(
            &fixture.client.base_urls[2],
            &fixture.tokens[2],
            repo,
            branch,
            workspace,
            &delete_target,
        )
        .await
        .expect("Failed to delete post from node3");

    let delete_timeout = Duration::from_secs(20);
    let delete_start = Instant::now();
    loop {
        let mut remaining = Vec::new();
        for (idx, (url, token)) in fixture
            .client
            .base_urls
            .iter()
            .zip(fixture.tokens.iter())
            .enumerate()
        {
            let node = fixture
                .client
                .get_node(url, token, repo, branch, workspace, &delete_target)
                .await
                .expect("Failed to check node deletion state");
            if node.is_some() {
                remaining.push(idx + 1);
            }
        }

        if remaining.is_empty() {
            break;
        }

        if delete_start.elapsed() > delete_timeout {
            panic!(
                "Deleted post {} is still present on nodes {:?}",
                delete_target, remaining
            );
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    verify_child_order_via_rest(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &user_paths[1],
    )
    .await
    .expect("Child order mismatch after deletion");

    // 4. Add a relation from a node that did not create the post and ensure it replicates
    println!("Adding cross-node relation and verifying replication...");
    let relation_source = &user_paths[2]; // Carol
    let relation_target = &cluster_post_paths[0]; // Alice's post (still present)

    let existing_relations = fixture
        .client
        .get_relations(
            &fixture.client.base_urls[0],
            &fixture.tokens[0],
            repo,
            branch,
            workspace,
            relation_source,
        )
        .await
        .expect("Failed to read baseline relations");
    let expected_relation_count = existing_relations.len() + 1;

    fixture
        .client
        .add_relation(
            &fixture.client.base_urls[1], // node2 mutates relation for user3
            &fixture.tokens[1],
            repo,
            branch,
            workspace,
            relation_source,
            relation_target,
            "likes",
        )
        .await
        .expect("Failed to add cross-node relation");

    let relation_timeout = Duration::from_secs(20);
    let relation_start = Instant::now();
    loop {
        match verify_relations_match(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            relation_source,
            expected_relation_count,
        )
        .await
        {
            Ok(_) => break,
            Err(err) => {
                if relation_start.elapsed() > relation_timeout {
                    panic!("Relations failed to replicate consistently: {}", err);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }

    println!("\n✅ Cross-node lifecycle test passed\n");
    fixture.teardown();
}

/// Test 7: Delete node and verify order updates
#[tokio::test]
#[ignore]
async fn test_delete_node_order_updates() {
    println!("\n=== Test: Delete Node and Verify Order Updates ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    let alice_path = &fixture.user_paths[0];
    let post_path = fixture.post_paths[0].clone();

    // Get initial children
    let initial_children = fixture
        .client
        .list_children(
            &fixture.client.base_urls[0],
            &fixture.tokens[0],
            fixture.repo(),
            fixture.branch(),
            fixture.workspace(),
            alice_path,
        )
        .await
        .expect("Failed to list children");

    println!("Alice has {} posts before deletion", initial_children.len());

    // Delete post on node1
    println!("\nDeleting post on node1...");
    fixture
        .client
        .delete_node(
            &fixture.client.base_urls[0],
            &fixture.tokens[0],
            fixture.repo(),
            fixture.branch(),
            fixture.workspace(),
            &post_path,
        )
        .await
        .expect("Failed to delete post");

    // Wait for replication
    println!("Waiting for deletion to replicate...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify node is deleted on all nodes
    for (idx, (url, token)) in fixture
        .client
        .base_urls
        .iter()
        .zip(&fixture.tokens)
        .enumerate()
    {
        let node = fixture
            .client
            .get_node(
                url,
                token,
                fixture.repo(),
                fixture.branch(),
                fixture.workspace(),
                &post_path,
            )
            .await
            .expect("Failed to check node");

        if node.is_some() {
            panic!("Node still exists on node{} after deletion", idx + 1);
        }
    }

    // Verify child order is still consistent
    println!("Verifying child order after deletion...");
    verify_child_order_via_rest(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        alice_path,
    )
    .await
    .expect("Child order inconsistent after deletion");

    println!("\n✅ Delete node test passed\n");
    fixture.teardown();
}

/// Test 8: Stress test with rapid operations
#[tokio::test]
#[ignore]
async fn test_child_order_stress_test() {
    println!("\n=== Test: Child Order Stress Test ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    let alice_path = &fixture.user_paths[0];

    // Create multiple posts rapidly on different nodes
    println!("Creating 9 posts rapidly across all nodes...");

    for i in 0..9 {
        let node_idx = i % 3;
        let post_id = format!("stress_post_{}", i);

        create_post(
            &fixture.client,
            &fixture.client.base_urls[node_idx],
            &fixture.tokens[node_idx],
            alice_path,
            &post_id,
            &format!("Stress Post {}", i),
            &format!("Created on node{} as part of stress test", node_idx + 1),
        )
        .await
        .expect("Failed to create post");

        // Small delay between posts
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Wait for all to replicate
    println!("Waiting for all posts to replicate...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Verify order consistency
    println!("Verifying child order consistency...");
    verify_child_order_via_rest(
        &fixture.client,
        &fixture.tokens,
        fixture.repo(),
        fixture.branch(),
        fixture.workspace(),
        alice_path,
    )
    .await
    .expect("Child order inconsistent after stress test");

    // Get final children for SQL verification
    let children = fixture
        .client
        .list_children(
            &fixture.client.base_urls[0],
            &fixture.tokens[0],
            fixture.repo(),
            fixture.branch(),
            fixture.workspace(),
            alice_path,
        )
        .await
        .expect("Failed to list children");

    let expected_ids: Vec<String> = children
        .iter()
        .filter_map(|c| c["id"].as_str().map(|s| s.to_string()))
        .collect();

    // Verify SQL returns same order (without ORDER BY)
    println!("Verifying SQL query returns same order...");
    verify_child_order_via_sql(
        &fixture.client,
        &fixture.tokens[0],
        &fixture.client.base_urls[0],
        fixture.repo(),
        fixture.workspace(),
        alice_path,
        &expected_ids,
    )
    .await
    .expect("SQL order doesn't match REST order");

    println!("\n✅ Stress test passed\n");
    fixture.teardown();
}

/// Test 9: WebSocket replication events (simplified)
#[tokio::test]
#[ignore]
async fn test_websocket_replication_events() {
    println!("\n=== Test: WebSocket Replication Events ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    // Note: This is a placeholder test. Full WebSocket testing would require:
    // 1. Connecting WebSocket client to a node
    // 2. Subscribing to events
    // 3. Creating a node on another node
    // 4. Receiving the replication event
    //
    // For now, we just verify the fixture setup works

    println!("WebSocket event test placeholder");
    println!("Full implementation would test event streaming");

    println!("\n✅ WebSocket test passed (placeholder)\n");
    fixture.teardown();
}

/// Test 10: Comprehensive cross-node operations (posts, updates, comments, relations, deletions)
#[tokio::test]
#[ignore]
async fn test_comprehensive_cross_node_operations() {
    println!("\n=== Test: Comprehensive Cross-Node Operations ===\n");

    let fixture = ClusterTestFixture::setup()
        .await
        .expect("Failed to setup cluster");

    let repo = fixture.repo();
    let branch = fixture.branch();
    let workspace = fixture.workspace();
    let user_paths = fixture.user_paths.clone();

    // === PHASE 1: Post Creation & Position Verification ===
    println!("Phase 1: Creating posts on different nodes and verifying position consistency...");

    let post_specs = vec![
        (
            0,
            0,
            "comprehensive_post_a",
            "Post A from Node1",
            "Created on node1",
        ),
        (
            1,
            1,
            "comprehensive_post_b",
            "Post B from Node2",
            "Created on node2",
        ),
        (
            2,
            2,
            "comprehensive_post_c",
            "Post C from Node3",
            "Created on node3",
        ),
    ];

    let mut post_paths = Vec::new();
    let mut post_ids = Vec::new();

    for (node_idx, user_idx, post_id, title, content) in post_specs {
        let author_path = &user_paths[user_idx];

        println!("  Creating {} on node{}...", post_id, node_idx + 1);
        create_post(
            &fixture.client,
            &fixture.client.base_urls[node_idx],
            &fixture.tokens[node_idx],
            author_path,
            post_id,
            title,
            content,
        )
        .await
        .expect(&format!("Failed to create {}", post_id));

        let post_path = format!("{}/{}", author_path, post_id);

        // Wait for replication
        wait_for_replication(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            &post_path,
            Duration::from_secs(10),
        )
        .await
        .expect(&format!("{} did not replicate", post_id));

        // Verify post appears at same position on all nodes
        println!("  Verifying {} position consistency...", post_id);
        verify_post_at_same_position(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            author_path,
            post_id,
        )
        .await
        .expect(&format!("{} position mismatch", post_id));

        post_paths.push(post_path);
        post_ids.push(post_id.to_string());
    }

    // Verify natural order matches across all nodes
    for user_path in &user_paths {
        verify_child_order_via_rest(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            user_path,
        )
        .await
        .expect("Child order mismatch after post creation");
    }

    println!("  ✓ Phase 1 complete: All posts created and position verified\n");

    // === PHASE 2: Cross-Node Post Updates ===
    println!("Phase 2: Updating posts from different nodes...");

    let update_specs = vec![
        (
            1,
            0,
            "Updated from node2",
            "Post A was created on node1, updated on node2",
        ),
        (
            2,
            1,
            "Updated from node3",
            "Post B was created on node2, updated on node3",
        ),
        (
            0,
            2,
            "Updated from node1",
            "Post C was created on node3, updated on node1",
        ),
    ];

    for (node_idx, post_idx, title, content) in update_specs {
        let post_path = &post_paths[post_idx];

        println!(
            "  Updating {} from node{}...",
            post_ids[post_idx],
            node_idx + 1
        );
        fixture
            .client
            .update_node(
                &fixture.client.base_urls[node_idx],
                &fixture.tokens[node_idx],
                repo,
                branch,
                workspace,
                post_path,
                json!({
                    "properties": {
                        "title": title,
                        "content": content,
                        "likeCount": (post_idx + 1) * 10
                    }
                }),
            )
            .await
            .expect("Failed to update post");

        tokio::time::sleep(Duration::from_secs(2)).await;

        // Verify properties match across all nodes
        verify_node_properties_match(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            post_path,
        )
        .await
        .expect("Properties mismatch after update");
    }

    println!("  ✓ Phase 2 complete: All posts updated cross-node\n");

    // === PHASE 3: Comment Operations ===
    println!("Phase 3: Creating and updating comments across nodes...");

    // Node1: Create comment on post A
    let comment_path_1 = format!("{}/comment_1", post_paths[0]);
    println!("  Node1: Creating comment on post A...");
    create_comment(
        &fixture.client,
        &fixture.client.base_urls[0],
        &fixture.tokens[0],
        &post_paths[0],
        "comment_1",
        "alice",
        "Great post! (comment from node1)",
    )
    .await
    .expect("Failed to create comment on node1");

    verify_comment_exists_on_all_nodes(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &comment_path_1,
        Duration::from_secs(10),
    )
    .await
    .expect("Comment 1 did not replicate");

    // Node2: Create comment on post B
    let comment_path_2 = format!("{}/comment_2", post_paths[1]);
    println!("  Node2: Creating comment on post B...");
    create_comment(
        &fixture.client,
        &fixture.client.base_urls[1],
        &fixture.tokens[1],
        &post_paths[1],
        "comment_2",
        "bob",
        "Interesting! (comment from node2)",
    )
    .await
    .expect("Failed to create comment on node2");

    verify_comment_exists_on_all_nodes(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &comment_path_2,
        Duration::from_secs(10),
    )
    .await
    .expect("Comment 2 did not replicate");

    // Node3: Update comment created on node1
    println!("  Node3: Updating comment created on node1...");
    fixture
        .client
        .update_node(
            &fixture.client.base_urls[2],
            &fixture.tokens[2],
            repo,
            branch,
            workspace,
            &comment_path_1,
            json!({
                "properties": {
                    "content": "Great post! (updated from node3)",
                    "likeCount": 5
                }
            }),
        )
        .await
        .expect("Failed to update comment from node3");

    tokio::time::sleep(Duration::from_secs(2)).await;

    verify_node_properties_match(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &comment_path_1,
    )
    .await
    .expect("Comment properties mismatch after update");

    println!("  ✓ Phase 3 complete: Comments created and updated cross-node\n");

    // === PHASE 4: Relation Operations ===
    println!("Phase 4: Creating relations from different nodes...");

    // Node1: Create "likes" relation from user1 to post B
    println!("  Node1: user1 likes post B...");
    fixture
        .client
        .add_relation(
            &fixture.client.base_urls[0],
            &fixture.tokens[0],
            repo,
            branch,
            workspace,
            &user_paths[0],
            &post_paths[1],
            "likes",
        )
        .await
        .expect("Failed to add relation on node1");

    verify_relation_exists_on_all_nodes(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &user_paths[0],
        &post_paths[1],
        "likes",
        Duration::from_secs(10),
    )
    .await
    .expect("Relation 1 did not replicate");

    // Node2: Create "follows" relation from user2 to user1
    println!("  Node2: user2 follows user1 (additional follow)...");
    fixture
        .client
        .add_relation(
            &fixture.client.base_urls[1],
            &fixture.tokens[1],
            repo,
            branch,
            workspace,
            &user_paths[1],
            &user_paths[0],
            "follows",
        )
        .await
        .expect("Failed to add relation on node2");

    verify_relation_exists_on_all_nodes(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &user_paths[1],
        &user_paths[0],
        "follows",
        Duration::from_secs(10),
    )
    .await
    .expect("Relation 2 did not replicate");

    // Node3: Create "likes" relation from user3 to post A
    println!("  Node3: user3 likes post A...");
    fixture
        .client
        .add_relation(
            &fixture.client.base_urls[2],
            &fixture.tokens[2],
            repo,
            branch,
            workspace,
            &user_paths[2],
            &post_paths[0],
            "likes",
        )
        .await
        .expect("Failed to add relation on node3");

    verify_relation_exists_on_all_nodes(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &user_paths[2],
        &post_paths[0],
        "likes",
        Duration::from_secs(10),
    )
    .await
    .expect("Relation 3 did not replicate");

    println!("  ✓ Phase 4 complete: All relations created and verified\n");

    // === PHASE 5: Relation Deletion ===
    println!("Phase 5: Deleting relations from different nodes...");

    // Node2: Remove relation created on node1
    println!("  Node2: Removing user1->postB likes relation (created on node1)...");
    fixture
        .client
        .remove_relation(
            &fixture.client.base_urls[1],
            &fixture.tokens[1],
            repo,
            branch,
            workspace,
            &user_paths[0],
            &post_paths[1],
            "likes",
        )
        .await
        .expect("Failed to remove relation on node2");

    verify_relation_deleted_on_all_nodes(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &user_paths[0],
        &post_paths[1],
        "likes",
        Duration::from_secs(10),
    )
    .await
    .expect("Relation deletion did not replicate");

    println!("  ✓ Phase 5 complete: Relation deletion verified\n");

    // === PHASE 6: Cross-Node Deletions ===
    println!("Phase 6: Deleting nodes from different cluster members...");

    // Node2: Delete post A (created on node1)
    println!("  Node2: Deleting post A (created on node1)...");
    fixture
        .client
        .delete_node(
            &fixture.client.base_urls[1],
            &fixture.tokens[1],
            repo,
            branch,
            workspace,
            &post_paths[0],
        )
        .await
        .expect("Failed to delete post from node2");

    // Verify deletion across all nodes
    let delete_start = Instant::now();
    let delete_timeout = Duration::from_secs(15);
    loop {
        let mut still_exists = Vec::new();
        for (idx, (url, token)) in fixture
            .client
            .base_urls
            .iter()
            .zip(fixture.tokens.iter())
            .enumerate()
        {
            let node = fixture
                .client
                .get_node(url, token, repo, branch, workspace, &post_paths[0])
                .await
                .expect("Failed to check node deletion");
            if node.is_some() {
                still_exists.push(idx + 1);
            }
        }

        if still_exists.is_empty() {
            break;
        }

        if delete_start.elapsed() > delete_timeout {
            panic!(
                "Post deletion did not replicate to all nodes. Still exists on: {:?}",
                still_exists
            );
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Node3: Delete comment (created on node1)
    println!("  Node3: Deleting comment (created on node1)...");
    fixture
        .client
        .delete_node(
            &fixture.client.base_urls[2],
            &fixture.tokens[2],
            repo,
            branch,
            workspace,
            &comment_path_1,
        )
        .await
        .expect("Failed to delete comment from node3");

    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify ordering remains consistent after deletions
    for user_path in &user_paths {
        verify_child_order_via_rest(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            user_path,
        )
        .await
        .expect("Child order mismatch after deletions");
    }

    println!("  ✓ Phase 6 complete: Cross-node deletions verified\n");

    println!("\n✅ Comprehensive cross-node operations test passed\n");
    println!("   - Posts created on each node ✓");
    println!("   - Posts updated from different nodes ✓");
    println!("   - Comments created and updated cross-node ✓");
    println!("   - Relations created from different nodes ✓");
    println!("   - Relations deleted from different nodes ✓");
    println!("   - Nodes deleted from different cluster members ✓");
    println!("   - Natural ordering maintained throughout ✓\n");

    fixture.teardown();
}

/// Test 11: 2-Node Cluster Test (isolating node3 issue)
#[tokio::test]
#[ignore]
async fn test_two_node_cluster() {
    println!("\n=== Test: 2-Node Cluster Operations ===\n");
    println!("Testing with 2 nodes to isolate if node3 causes replication issues\n");

    let fixture = ClusterTestFixture::setup_with_nodes(2)
        .await
        .expect("Failed to setup 2-node cluster");

    println!("\n✓ Initial 2-node cluster setup complete");

    let repo = fixture.repo();
    let branch = fixture.branch();
    let workspace = fixture.workspace();
    let user_paths = fixture.user_paths.clone();

    println!("\n✓ 2-node cluster with restart complete!");
    println!("  - {} users replicated", user_paths.len());
    println!("  - {} initial posts replicated", fixture.post_paths.len());

    // Test creating a post on each node
    println!("\nTesting post creation on both nodes...");

    // Create post on node1
    println!("  Creating post on node1...");
    let post_a_id = "two_node_post_a";
    create_post(
        &fixture.client,
        &fixture.client.base_urls[0],
        &fixture.tokens[0],
        &user_paths[0],
        post_a_id,
        "Post from Node1",
        "Created on node1 in 2-node cluster",
    )
    .await
    .expect("Failed to create post on node1");

    let post_a_path = format!("{}/{}", user_paths[0], post_a_id);

    wait_for_replication(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &post_a_path,
        Duration::from_secs(10),
    )
    .await
    .expect("Post A did not replicate");

    println!("    ✓ Post A replicated to node2");

    // Create post on node2
    println!("  Creating post on node2...");
    let post_b_id = "two_node_post_b";
    create_post(
        &fixture.client,
        &fixture.client.base_urls[1],
        &fixture.tokens[1],
        &user_paths[1],
        post_b_id,
        "Post from Node2",
        "Created on node2 in 2-node cluster",
    )
    .await
    .expect("Failed to create post on node2");

    let post_b_path = format!("{}/{}", user_paths[1], post_b_id);

    wait_for_replication(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &post_b_path,
        Duration::from_secs(10),
    )
    .await
    .expect("Post B did not replicate");

    println!("    ✓ Post B replicated to node1");

    // Test relation creation
    println!("\nTesting relation operations...");
    println!("  Creating relation from node1...");
    fixture
        .client
        .add_relation(
            &fixture.client.base_urls[0],
            &fixture.tokens[0],
            repo,
            branch,
            workspace,
            &post_a_path,
            &post_b_path,
            "references",
        )
        .await
        .expect("Failed to create relation");

    tokio::time::sleep(Duration::from_secs(2)).await;

    verify_relation_exists_on_all_nodes(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &post_a_path,
        &post_b_path,
        "references",
        Duration::from_secs(10),
    )
    .await
    .expect("Relation did not replicate");

    println!("    ✓ Relation replicated");

    // Test relation deletion
    println!("  Deleting relation from node2...");
    fixture
        .client
        .remove_relation(
            &fixture.client.base_urls[1],
            &fixture.tokens[1],
            repo,
            branch,
            workspace,
            &post_a_path,
            &post_b_path,
            "references",
        )
        .await
        .expect("Failed to remove relation");

    tokio::time::sleep(Duration::from_secs(2)).await;

    verify_relation_deleted_on_all_nodes(
        &fixture.client,
        &fixture.tokens,
        repo,
        branch,
        workspace,
        &post_a_path,
        &post_b_path,
        "references",
        Duration::from_secs(10),
    )
    .await
    .expect("Relation deletion did not replicate");

    println!("    ✓ Relation deletion replicated (or deletion not working yet)");

    println!("\n✅ 2-node cluster test passed");
    println!("   - Setup successful without node3");
    println!("   - Post creation works on both nodes");
    println!("   - Replication works between 2 nodes");
    println!("   - Relations can be created");
    println!("   - Check if relation deletion works\n");

    fixture.teardown();
}

/// Helper function: Generic N-Node Cluster Test
/// Tests cluster replication with any number of nodes (2, 3, 5, etc.)
async fn test_n_node_cluster_impl(num_nodes: usize) {
    println!("\n=== Test: {}-Node Cluster Operations ===\n", num_nodes);
    println!(
        "Testing with {} nodes for complete mesh replication\n",
        num_nodes
    );

    let fixture = ClusterTestFixture::setup_with_nodes(num_nodes)
        .await
        .expect(&format!("Failed to setup {}-node cluster", num_nodes));

    println!("\n✓ Initial {}-node cluster setup complete", num_nodes);

    let repo = fixture.repo();
    let branch = fixture.branch();
    let workspace = fixture.workspace();
    let user_paths = fixture.user_paths.clone();

    println!("\n✓ {}-node cluster ready!", num_nodes);
    println!("  - {} users replicated", user_paths.len());
    println!("  - {} initial posts replicated", fixture.post_paths.len());

    // Test 1: Create a post on each node and verify it replicates to all others
    println!("\nTesting post creation on each node...");

    let mut created_posts = Vec::new();

    for node_idx in 0..num_nodes {
        println!("  Creating post on node{}...", node_idx + 1);
        let post_id = format!("n{}_node_post_{}", num_nodes, node_idx + 1);
        let user_idx = node_idx % user_paths.len();

        create_post(
            &fixture.client,
            &fixture.client.base_urls[node_idx],
            &fixture.tokens[node_idx],
            &user_paths[user_idx],
            &post_id,
            &format!("Post from Node{}", node_idx + 1),
            &format!(
                "Created on node{} in {}-node cluster",
                node_idx + 1,
                num_nodes
            ),
        )
        .await
        .expect(&format!("Failed to create post on node{}", node_idx + 1));

        let post_path = format!("{}/{}", user_paths[user_idx], post_id);
        created_posts.push((post_path.clone(), node_idx + 1));

        // Wait for replication to all nodes
        wait_for_replication(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            &post_path,
            Duration::from_secs(10),
        )
        .await
        .expect(&format!(
            "Post from node{} did not replicate to all nodes",
            node_idx + 1
        ));

        println!(
            "    ✓ Post from node{} replicated to all {} nodes",
            node_idx + 1,
            num_nodes
        );
    }

    println!(
        "\n  ✓ All {} posts created and replicated successfully",
        num_nodes
    );

    // Test 2: Create relations between posts from different nodes
    println!("\nTesting relation operations across nodes...");

    if created_posts.len() >= 2 {
        let (source_post, source_node) = &created_posts[0];
        let (target_post, target_node) = &created_posts[1];

        println!(
            "  Creating relation from node{} (post on node{} -> post on node{})...",
            source_node, source_node, target_node
        );

        fixture
            .client
            .add_relation(
                &fixture.client.base_urls[0],
                &fixture.tokens[0],
                repo,
                branch,
                workspace,
                source_post,
                target_post,
                "references",
            )
            .await
            .expect("Failed to create relation");

        tokio::time::sleep(Duration::from_secs(2)).await;

        verify_relation_exists_on_all_nodes(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            source_post,
            target_post,
            "references",
            Duration::from_secs(10),
        )
        .await
        .expect("Relation did not replicate to all nodes");

        println!("    ✓ Relation replicated to all {} nodes", num_nodes);

        // Test relation deletion from a different node
        let delete_node_idx = if num_nodes > 2 { 2 } else { 1 };
        println!("  Deleting relation from node{}...", delete_node_idx + 1);

        fixture
            .client
            .remove_relation(
                &fixture.client.base_urls[delete_node_idx],
                &fixture.tokens[delete_node_idx],
                repo,
                branch,
                workspace,
                source_post,
                target_post,
                "references",
            )
            .await
            .expect("Failed to remove relation");

        tokio::time::sleep(Duration::from_secs(2)).await;

        verify_relation_deleted_on_all_nodes(
            &fixture.client,
            &fixture.tokens,
            repo,
            branch,
            workspace,
            source_post,
            target_post,
            "references",
            Duration::from_secs(10),
        )
        .await
        .expect("Relation deletion did not replicate to all nodes");

        println!(
            "    ✓ Relation deletion replicated to all {} nodes",
            num_nodes
        );
    }

    println!("\n✅ {}-node cluster test passed", num_nodes);
    println!("   - Setup successful with {} nodes", num_nodes);
    println!("   - Post creation works on all nodes");
    println!("   - Replication works across all {} nodes", num_nodes);
    println!("   - Relations can be created cross-node");
    println!("   - Relation deletion replicates correctly\n");

    fixture.teardown();
}

/// Test 12: 3-Node Cluster Test
#[tokio::test]
#[ignore]
async fn test_three_node_cluster() {
    test_n_node_cluster_impl(3).await;
}

/// Test 13: 5-Node Cluster Test
#[tokio::test]
#[ignore]
async fn test_five_node_cluster() {
    test_n_node_cluster_impl(5).await;
}
