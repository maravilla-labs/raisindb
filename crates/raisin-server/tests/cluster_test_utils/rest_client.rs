// REST API client for cluster testing

use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

/// REST client for interacting with RaisinDB cluster nodes
pub struct RestClient {
    client: Client,
    pub base_urls: Vec<String>,
}

impl RestClient {
    /// Create a new REST client for the cluster
    pub fn new(base_urls: Vec<String>) -> Self {
        Self {
            client: Client::new(),
            base_urls,
        }
    }

    /// Authenticate to a node and get a JWT token
    pub async fn authenticate(
        &self,
        node_url: &str,
        tenant_id: &str,
        username: &str,
        password: &str,
    ) -> Result<String> {
        let auth_url = format!("{}/api/raisindb/sys/{}/auth", node_url, tenant_id);

        let response = self
            .client
            .post(&auth_url)
            .json(&json!({
                "username": username,
                "password": password,
                "interface": "console"
            }))
            .send()
            .await
            .context("Authentication request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Authentication failed with status {}: {}", status, body);
        }

        let auth_response: Value = response
            .json()
            .await
            .context("Failed to parse auth response")?;

        auth_response["token"]
            .as_str()
            .map(|s| s.to_string())
            .context("No token in auth response")
    }

    /// Create a repository
    pub async fn create_repository(
        &self,
        node_url: &str,
        token: &str,
        repo_id: &str,
    ) -> Result<()> {
        let url = format!("{}/api/repositories", node_url); // Note: plural "repositories"

        let response = self
            .client
            .post(&url) // POST, not PUT
            .bearer_auth(token)
            .json(&json!({
                "repo_id": repo_id, // repo_id goes in the body
                "description": format!("Test repository {}", repo_id),
                "default_branch": "main"
            }))
            .send()
            .await
            .context("Create repository request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Create repository failed with status {}: {}", status, body);
        }

        Ok(())
    }

    /// Create a workspace
    pub async fn create_workspace(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        workspace: &str,
    ) -> Result<()> {
        let url = format!("{}/api/workspaces/{}/{}", node_url, repo, workspace);

        let response = self
            .client
            .put(&url)
            .bearer_auth(token)
            .json(&json!({
                "name": workspace,
                "description": format!("Test workspace {}", workspace),
                "allowed_node_types": ["app:SocialUser", "app:Post", "app:Comment", "raisin:Folder"],
                "allowed_root_node_types": ["app:SocialUser", "app:Post", "raisin:Folder"],
                "depends_on": [],
                "config": {
                    "default_branch": "main",
                    "node_type_pins": {}
                }
            }))
            .send()
            .await
            .context("Create workspace request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Create workspace failed with status {}: {}", status, body);
        }

        Ok(())
    }

    /// Create a node
    pub async fn create_node(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        node_data: Value,
    ) -> Result<Value> {
        let url = format!(
            "{}/api/repository/{}/{}/head/{}/{}",
            node_url, repo, branch, workspace, parent_path
        );

        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(&json!({ "node": node_data }))
            .send()
            .await
            .context("Create node request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Create node failed with status {}: {}", status, body);
        }

        response.json().await.context("Failed to parse response")
    }

    /// Get a node by path
    pub async fn get_node(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> Result<Option<Value>> {
        let url = format!(
            "{}/api/repository/{}/{}/head/{}/{}",
            node_url, repo, branch, workspace, path
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("Get node request failed")?;

        match response.status() {
            StatusCode::OK => {
                let node = response.json().await.context("Failed to parse node")?;
                Ok(Some(node))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => {
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("Get node failed with status {}: {}", status, body)
            }
        }
    }

    /// Get a node by ID
    pub async fn get_node_by_id(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Option<Value>> {
        let url = format!(
            "{}/api/repository/{}/{}/head/{}/$ref/{}",
            node_url, repo, branch, workspace, node_id
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("Get node by ID request failed")?;

        match response.status() {
            StatusCode::OK => {
                let node = response.json().await.context("Failed to parse node")?;
                Ok(Some(node))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => {
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("Get node by ID failed with status {}: {}", status, body)
            }
        }
    }

    /// Update a node (using the raisin:cmd/save endpoint)
    pub async fn update_node(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        node_data: Value,
    ) -> Result<()> {
        let url = format!(
            "{}/api/repository/{}/{}/head/{}/{}/raisin:cmd/save",
            node_url, repo, branch, workspace, path
        );

        let properties = node_data
            .get("properties")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let node_id = path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(path)
            .to_string();

        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(&json!({
                "actor": "cluster-test",
                "message": "cluster test update",
                "operations": [
                    {
                        "type": "update",
                        "node_id": node_id,
                        "properties": properties
                    }
                ]
            }))
            .send()
            .await
            .context("Update node request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Update node failed with status {}: {}", status, body);
        }

        Ok(())
    }

    /// Delete a node (using the raisin:cmd/delete endpoint)
    pub async fn delete_node(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/api/repository/{}/{}/head/{}/{}/raisin:cmd/delete",
            node_url, repo, branch, workspace, path
        );

        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(&json!({
                "actor": "cluster-test",
                "message": "cluster test delete"
            }))
            .send()
            .await
            .context("Delete node request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Delete node failed with status {}: {}", status, body);
        }

        Ok(())
    }

    /// List children of a node (returns natural order from fragmented index)
    pub async fn list_children(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Vec<Value>> {
        let url = format!(
            "{}/api/repository/{}/{}/head/{}/{}",
            node_url, repo, branch, workspace, parent_path
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("List children request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("List children failed with status {}: {}", status, body);
        }

        let parent: Value = response.json().await.context("Failed to parse parent")?;

        // Extract children array
        if let Some(children) = parent["children"].as_array() {
            Ok(children.clone())
        } else {
            Ok(Vec::new())
        }
    }

    /// Add a relation between two nodes
    pub async fn add_relation(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        source_path: &str,
        target_path: &str,
        relation_type: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/api/repository/{}/{}/head/{}/{}/raisin:cmd/add-relation",
            node_url, repo, branch, workspace, source_path
        );

        let target_path = if target_path.starts_with('/') {
            target_path.to_string()
        } else {
            format!("/{}", target_path)
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(&json!({
                "targetWorkspace": workspace,
                "targetPath": target_path,
                "relationType": relation_type
            }))
            .send()
            .await
            .context("Add relation request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Add relation failed with status {}: {}", status, body);
        }

        Ok(())
    }

    /// Remove a relation between two nodes
    pub async fn remove_relation(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        source_path: &str,
        target_path: &str,
        relation_type: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/api/repository/{}/{}/head/{}/{}/raisin:cmd/remove-relation",
            node_url, repo, branch, workspace, source_path
        );

        let target_path = if target_path.starts_with('/') {
            target_path.to_string()
        } else {
            format!("/{}", target_path)
        };

        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(&json!({
                "targetWorkspace": workspace,
                "targetPath": target_path,
                "relationType": relation_type
            }))
            .send()
            .await
            .context("Remove relation request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Remove relation failed with status {}: {}", status, body);
        }

        Ok(())
    }

    /// Get relations for a node
    pub async fn get_relations(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        node_path: &str,
    ) -> Result<Vec<Value>> {
        let url = format!(
            "{}/api/repository/{}/{}/head/{}/{}/raisin:cmd/relations",
            node_url, repo, branch, workspace, node_path
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("Get relations request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Get relations failed with status {}: {}", status, body);
        }

        let payload: Value = response.json().await.context("Failed to parse relations")?;
        let outgoing = payload["outgoing"]
            .as_array()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Relations response missing 'outgoing' array"))?;
        Ok(outgoing)
    }

    /// Create a NodeType
    pub async fn create_node_type(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        node_type_data: Value,
        commit_message: &str,
    ) -> Result<()> {
        let url = format!("{}/api/management/{}/{}/nodetypes", node_url, repo, branch);

        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(&json!({
                "node_type": node_type_data,
                "commit": {
                    "message": commit_message,
                    "actor": "cluster-test"
                }
            }))
            .send()
            .await
            .context("Create NodeType request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Create NodeType failed with status {}: {}", status, body);
        }

        Ok(())
    }

    /// Get a NodeType by name
    pub async fn get_node_type(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        branch: &str,
        node_type_name: &str,
    ) -> Result<Option<Value>> {
        let url = format!(
            "{}/api/management/{}/{}/nodetypes/{}",
            node_url, repo, branch, node_type_name
        );

        let response = self
            .client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("Get NodeType request failed")?;

        match response.status() {
            StatusCode::OK => {
                let node_type = response.json().await.context("Failed to parse NodeType")?;
                Ok(Some(node_type))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => {
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("Get NodeType failed with status {}: {}", status, body)
            }
        }
    }

    /// Execute SQL query (without ORDER BY to test natural order)
    pub async fn execute_sql(
        &self,
        node_url: &str,
        token: &str,
        repo: &str,
        sql: &str,
        params: Vec<Value>,
    ) -> Result<Value> {
        let url = format!("{}/api/sql/{}", node_url, repo);

        let response = self
            .client
            .post(&url)
            .bearer_auth(token)
            .json(&json!({
                "sql": sql,
                "params": params
            }))
            .send()
            .await
            .context("SQL query request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("SQL query failed with status {}: {}", status, body);
        }

        response.json().await.context("Failed to parse SQL result")
    }
}
