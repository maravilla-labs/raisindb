// Social feed demo schema initialization for cluster testing

use super::rest_client::RestClient;
use anyhow::{Context, Result};
use serde_json::json;

pub const SOCIAL_FEED_REPO: &str = "social_feed_demo";
pub const SOCIAL_FEED_BRANCH: &str = "main";
pub const SOCIAL_FEED_WORKSPACE: &str = "social";
pub const SOCIAL_FEED_NODE_TYPES: [&str; 3] = ["app:SocialUser", "app:Post", "app:Comment"];

/// Initialize the social feed schema on a node
///
/// This creates:
/// - Repository "social_feed_demo"
/// - Workspace "social"
/// - Node types: SocialUser, Post, Comment
pub async fn init_social_feed_schema(
    client: &RestClient,
    node_url: &str,
    token: &str,
) -> Result<()> {
    println!("Initializing social feed schema on {}", node_url);

    // Step 1: Create repository
    client
        .create_repository(node_url, token, SOCIAL_FEED_REPO)
        .await
        .context("Failed to create repository")?;

    println!("  Repository '{}' created", SOCIAL_FEED_REPO);

    // Step 2: Create social workspace
    println!("  Creating workspace '{}'...", SOCIAL_FEED_WORKSPACE);
    client
        .create_workspace(node_url, token, SOCIAL_FEED_REPO, SOCIAL_FEED_WORKSPACE)
        .await
        .context("Failed to create workspace")?;
    println!("  Workspace '{}' created", SOCIAL_FEED_WORKSPACE);

    // Step 3: Create NodeTypes (use namespace format for validation)
    println!("  Creating NodeTypes...");

    // app:SocialUser NodeType
    let user_node_type = json!({
        "name": "app:SocialUser",
        "description": "A user in the social network",
        "properties": [
            {
                "name": "username",
                "type": "String",
                "required": true,
                "unique": true
            },
            {
                "name": "displayName",
                "type": "String",
                "required": true
            },
            {
                "name": "bio",
                "type": "String"
            }
        ],
        "allowed_children": []
    });

    client
        .create_node_type(
            node_url,
            token,
            SOCIAL_FEED_REPO,
            SOCIAL_FEED_BRANCH,
            user_node_type,
            "Create SocialUser NodeType",
        )
        .await
        .context("Failed to create app:SocialUser NodeType")?;

    // Small delay to allow NodeType to be fully persisted
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    println!("    ✓ app:SocialUser NodeType created");

    // app:Post NodeType
    let post_node_type = json!({
        "name": "app:Post",
        "description": "A social media post",
        "properties": [
            {
                "name": "content",
                "type": "String",
                "required": true
            },
            {
                "name": "likeCount",
                "type": "Number",
                "default": 0
            },
            {
                "name": "commentCount",
                "type": "Number",
                "default": 0
            }
        ],
        "allowed_children": ["app:Comment"]
    });

    client
        .create_node_type(
            node_url,
            token,
            SOCIAL_FEED_REPO,
            SOCIAL_FEED_BRANCH,
            post_node_type,
            "Create Post NodeType",
        )
        .await
        .context("Failed to create app:Post NodeType")?;

    println!("    ✓ app:Post NodeType created");

    // app:Comment NodeType
    let comment_node_type = json!({
        "name": "app:Comment",
        "description": "A comment on a post",
        "properties": [
            {
                "name": "content",
                "type": "String",
                "required": true
            },
            {
                "name": "likeCount",
                "type": "Number",
                "default": 0
            }
        ],
        "allowed_children": []
    });

    client
        .create_node_type(
            node_url,
            token,
            SOCIAL_FEED_REPO,
            SOCIAL_FEED_BRANCH,
            comment_node_type,
            "Create Comment NodeType",
        )
        .await
        .context("Failed to create app:Comment NodeType")?;

    println!("    ✓ app:Comment NodeType created");

    // Wait a moment for event handlers to complete
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    println!("  Schema initialization complete");

    Ok(())
}

/// Create demo users in the social feed
///
/// Returns a vector of user IDs in order: [alice_id, bob_id, carol_id]
pub async fn create_demo_users(
    client: &RestClient,
    node_url: &str,
    token: &str,
) -> Result<Vec<(String, String)>> {
    println!("Creating demo users...");

    // Ensure /users folder exists
    let users_folder = json!({
        "id": "users",
        "name": "Users",
        "node_type": "raisin:Folder",
        "properties": {}
    });
    let _ = client
        .create_node(
            node_url,
            token,
            SOCIAL_FEED_REPO,
            SOCIAL_FEED_BRANCH,
            SOCIAL_FEED_WORKSPACE,
            "",
            users_folder,
        )
        .await;

    let users = vec![
        ("alice", "Alice Anderson", "alice@example.com"),
        ("bob", "Bob Builder", "bob@example.com"),
        ("carol", "Carol Chen", "carol@example.com"),
    ];

    let mut user_records = Vec::new();

    for (username, full_name, email) in users {
        let user_data = json!({
            "id": username,
            "name": username,
            "node_type": "app:SocialUser",
            "properties": {
                "username": username,
                "displayName": full_name,
                "bio": format!("I am {}!", full_name)
            }
        });

        client
            .create_node(
                node_url,
                token,
                SOCIAL_FEED_REPO,
                SOCIAL_FEED_BRANCH,
                SOCIAL_FEED_WORKSPACE,
                "users",
                user_data,
            )
            .await
            .with_context(|| format!("Failed to create user {}", username))?;

        user_records.push((username.to_string(), format!("users/{}", username)));
        println!("  Created user: {}", username);
    }

    Ok(user_records)
}

/// Create initial posts in the social feed
///
/// Returns a vector of post IDs
pub async fn create_initial_posts(
    client: &RestClient,
    node_url: &str,
    token: &str,
    users: &[(String, String)],
) -> Result<Vec<(String, String)>> {
    println!("Creating initial posts...");

    if users.len() < 3 {
        anyhow::bail!("Need at least 3 users to create initial posts");
    }

    let posts = vec![
        (
            "post_1",
            0usize,
            "Hello World!",
            "This is my first post on this social network!",
        ),
        (
            "post_2",
            1usize,
            "Building Things",
            "Just finished a great project. Feeling accomplished!",
        ),
        (
            "post_3",
            2usize,
            "Coffee Time",
            "Morning coffee is the best. What's your favorite brew?",
        ),
    ];

    let mut post_records = Vec::new();

    for (post_id, user_idx, title, content) in posts {
        let (author_id, author_path) = users
            .get(user_idx)
            .context("User index out of bounds while creating posts")?;
        let post_data = json!({
            "id": post_id,
            "name": post_id,
            "node_type": "app:Post",
            "properties": {
                "title": title,
                "content": content,
                "likeCount": 0,
                "commentCount": 0
            }
        });

        client
            .create_node(
                node_url,
                token,
                SOCIAL_FEED_REPO,
                SOCIAL_FEED_BRANCH,
                SOCIAL_FEED_WORKSPACE,
                author_path,
                post_data,
            )
            .await
            .with_context(|| format!("Failed to create post {}", post_id))?;

        post_records.push((post_id.to_string(), format!("{}/{}", author_path, post_id)));
        println!("  Created post: {} by {}", post_id, author_id);
    }

    Ok(post_records)
}

/// Add follow relationships between users
pub async fn add_follow_relationships(
    client: &RestClient,
    node_url: &str,
    token: &str,
    user_paths: &[String],
) -> Result<()> {
    println!("Adding follow relationships...");

    if user_paths.len() < 3 {
        anyhow::bail!("Need at least 3 users to create follow relationships");
    }

    // Alice follows Bob and Carol
    client
        .add_relation(
            node_url,
            token,
            SOCIAL_FEED_REPO,
            SOCIAL_FEED_BRANCH,
            SOCIAL_FEED_WORKSPACE,
            &user_paths[0], // alice
            &user_paths[1], // bob
            "follows",
        )
        .await
        .context("Failed to create alice -> bob follow")?;

    client
        .add_relation(
            node_url,
            token,
            SOCIAL_FEED_REPO,
            SOCIAL_FEED_BRANCH,
            SOCIAL_FEED_WORKSPACE,
            &user_paths[0], // alice
            &user_paths[2], // carol
            "follows",
        )
        .await
        .context("Failed to create alice -> carol follow")?;

    // Bob follows Carol
    client
        .add_relation(
            node_url,
            token,
            SOCIAL_FEED_REPO,
            SOCIAL_FEED_BRANCH,
            SOCIAL_FEED_WORKSPACE,
            &user_paths[1], // bob
            &user_paths[2], // carol
            "follows",
        )
        .await
        .context("Failed to create bob -> carol follow")?;

    println!("  Follow relationships created");
    Ok(())
}

/// Helper to create a comment on a post
pub async fn create_comment(
    client: &RestClient,
    node_url: &str,
    token: &str,
    post_path: &str,
    comment_id: &str,
    author_id: &str,
    content: &str,
) -> Result<String> {
    let comment_data = json!({
        "id": comment_id,
        "name": format!("Comment by {}", author_id),
        "node_type": "app:Comment",
        "properties": {
            "content": content,
            "likeCount": 0
        }
    });

    client
        .create_node(
            node_url,
            token,
            SOCIAL_FEED_REPO,
            SOCIAL_FEED_BRANCH,
            SOCIAL_FEED_WORKSPACE,
            post_path,
            comment_data,
        )
        .await
        .context("Failed to create comment")?;

    Ok(comment_id.to_string())
}

/// Helper to create a new post
pub async fn create_post(
    client: &RestClient,
    node_url: &str,
    token: &str,
    author_path: &str,
    post_id: &str,
    title: &str,
    content: &str,
) -> Result<String> {
    let post_data = json!({
        "id": post_id,
        "name": post_id,
        "node_type": "app:Post",
        "properties": {
            "title": title,
            "content": content,
            "likeCount": 0,
            "commentCount": 0
        }
    });

    client
        .create_node(
            node_url,
            token,
            SOCIAL_FEED_REPO,
            SOCIAL_FEED_BRANCH,
            SOCIAL_FEED_WORKSPACE,
            author_path,
            post_data,
        )
        .await
        .context("Failed to create post")?;

    Ok(post_id.to_string())
}
