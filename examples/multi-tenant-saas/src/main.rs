//! Multi-Tenant SaaS Example for RaisinDB
//!
//! This example demonstrates how to build a multi-tenant SaaS application
//! using RaisinDB with:
//!
//! - Subdomain-based tenant resolution
//! - Per-tenant rate limiting
//! - Service tier management (Free/Pro/Enterprise)
//! - Isolated data storage per tenant
//!
//! ## Running the Example
//!
//! ```bash
//! cargo run --example multi-tenant-saas
//! ```
//!
//! ## Testing with curl
//!
//! ```bash
//! # Create a node for tenant "acme" in production
//! curl -H "Host: acme.localhost:3000" \
//!      -H "Content-Type: application/json" \
//!      -X POST http://localhost:3000/api/nodes \
//!      -d '{"name":"test","node_type":"global:Folder"}'
//!
//! # Get nodes for tenant "acme"
//! curl -H "Host: acme.localhost:3000" \
//!      http://localhost:3000/api/nodes
//! ```

mod middleware;
mod tier_provider;

use axum::{
    extract::{Host, Path, State},
    http::StatusCode,
    middleware as axum_middleware,
    routing::{get, post},
    Json, Router,
};
use raisin_context::{TenantContext, TenantResolver};
use raisin_core::NodeService;
use raisin_models::nodes::Node;
// StorageExt removed - scoping now handled at service level
use raisin_storage_rocks::RocksStorage;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

use crate::middleware::TenantMiddleware;
use crate::tier_provider::SimpleTierProvider;

/// Application state shared across all handlers
#[derive(Clone)]
struct AppState {
    storage: Arc<RocksStorage>,
    tier_provider: Arc<SimpleTierProvider>,
}

/// Extract tenant context from request
fn extract_tenant_from_host(host: &str) -> Option<TenantContext> {
    // Parse subdomain: "acme.localhost:3000" -> "acme"
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let subdomain = parts[0];

    // Skip non-tenant subdomains
    if subdomain == "www" || subdomain == "api" || subdomain.is_empty() {
        return None;
    }

    // For this example, we always use "production" deployment
    Some(TenantContext::new(subdomain, "production"))
}

/// Health check endpoint
async fn health() -> &'static str {
    "OK"
}

/// Create a node for the current tenant
async fn create_node(
    Host(host): Host,
    State(state): State<AppState>,
    Json(mut node): Json<Node>,
) -> Result<Json<Node>, StatusCode> {
    let tenant_ctx = extract_tenant_from_host(&host).ok_or(StatusCode::BAD_REQUEST)?;

    // Check rate limits
    let tier = state.tier_provider.get_tier(tenant_ctx.tenant_id()).await;
    // TODO: Actually check rate limits here

    // Create scoped service for this tenant with full context
    let workspace = "default";
    let service = NodeService::new_with_context(
        state.storage.clone(),
        tenant_ctx.tenant_id().to_string(),
        "main".to_string(),      // repository
        "main".to_string(),      // branch
        workspace.to_string(),
    );

    // Create the node
    let created = service
        .add_node("/", node)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(created))
}

/// List nodes for the current tenant
async fn list_nodes(
    Host(host): Host,
    State(state): State<AppState>,
) -> Result<Json<Vec<Node>>, StatusCode> {
    let tenant_ctx = extract_tenant_from_host(&host).ok_or(StatusCode::BAD_REQUEST)?;

    // Create scoped service for this tenant with full context
    let workspace = "default";
    let service = NodeService::new_with_context(
        state.storage.clone(),
        tenant_ctx.tenant_id().to_string(),
        "main".to_string(),      // repository
        "main".to_string(),      // branch
        workspace.to_string(),
    );

    let nodes = service
        .list_all()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(nodes))
}

/// Get node by ID for the current tenant
async fn get_node(
    Host(host): Host,
    State(state): State<AppState>,
    Path(node_id): Path<String>,
) -> Result<Json<Node>, StatusCode> {
    let tenant_ctx = extract_tenant_from_host(&host).ok_or(StatusCode::BAD_REQUEST)?;

    // Create scoped service for this tenant with full context
    let workspace = "default";
    let service = NodeService::new_with_context(
        state.storage.clone(),
        tenant_ctx.tenant_id().to_string(),
        "main".to_string(),      // repository
        "main".to_string(),      // branch
        workspace.to_string(),
    );

    let node = service
        .get(&node_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(node))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🚀 Starting Multi-Tenant SaaS Example");
    println!("   Using RocksDB for storage");
    println!("   Tenant isolation: subdomain-based");
    println!();

    // Initialize storage
    let storage = Arc::new(RocksStorage::open("./data/multi-tenant")?);
    println!("✓ Storage initialized at ./data/multi-tenant");

    // Initialize tier provider
    let tier_provider = Arc::new(SimpleTierProvider::new());
    println!("✓ Tier provider initialized");

    let state = AppState {
        storage,
        tier_provider,
    };

    // Build router
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/nodes", post(create_node).get(list_nodes))
        .route("/api/nodes/:id", get(get_node))
        .with_state(state)
        .layer(CorsLayer::permissive());

    println!();
    println!("🌐 Server listening on http://localhost:3000");
    println!();
    println!("📝 Example requests:");
    println!("   # Create node for 'acme' tenant:");
    println!("   curl -H 'Host: acme.localhost:3000' \\");
    println!("        -H 'Content-Type: application/json' \\");
    println!("        -X POST http://localhost:3000/api/nodes \\");
    println!("        -d '{{\"name\":\"test\",\"node_type\":\"global:Folder\"}}'");
    println!();
    println!("   # List nodes for 'acme' tenant:");
    println!("   curl -H 'Host: acme.localhost:3000' \\");
    println!("        http://localhost:3000/api/nodes");
    println!();

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}
