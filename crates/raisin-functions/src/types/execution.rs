// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Execution context and result types

use chrono::{DateTime, Utc};
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Context provided to function execution
#[derive(Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    /// Unique execution ID
    pub execution_id: String,

    /// Tenant ID
    pub tenant_id: String,

    /// Repository ID
    pub repo_id: String,

    /// Branch name
    pub branch: String,

    /// Workspace (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,

    /// Actor performing the operation (user ID or "system")
    pub actor: String,

    /// Trigger name that invoked this execution (if triggered)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_name: Option<String>,

    /// Event data (if event-triggered)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_data: Option<serde_json::Value>,

    /// HTTP request data (if HTTP-triggered)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_request: Option<HttpRequestData>,

    /// Input parameters passed to the function
    pub input: serde_json::Value,

    /// Custom context metadata
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,

    /// When execution started
    pub started_at: DateTime<Utc>,

    /// Authentication context for RLS filtering.
    /// If None, operations run without auth (system context behavior for backwards compat).
    #[serde(skip)]
    pub auth_context: Option<AuthContext>,

    /// Whether this function is allowed to escalate to admin context via raisin.asAdmin().
    /// Set from function metadata `requiresAdmin: true`.
    #[serde(default)]
    pub allows_admin_escalation: bool,

    /// Optional log emitter for real-time log streaming to SSE clients.
    /// When set, console.log/warn/error calls will be streamed in real-time.
    #[serde(skip)]
    pub log_emitter: Option<raisin_storage::LogEmitter>,
}

impl std::fmt::Debug for ExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionContext")
            .field("execution_id", &self.execution_id)
            .field("tenant_id", &self.tenant_id)
            .field("repo_id", &self.repo_id)
            .field("branch", &self.branch)
            .field("workspace_id", &self.workspace_id)
            .field("actor", &self.actor)
            .field("trigger_name", &self.trigger_name)
            .field("event_data", &self.event_data)
            .field("http_request", &self.http_request)
            .field("input", &self.input)
            .field("metadata", &self.metadata)
            .field("started_at", &self.started_at)
            .field("auth_context", &self.auth_context)
            .field("allows_admin_escalation", &self.allows_admin_escalation)
            .field(
                "log_emitter",
                &self.log_emitter.as_ref().map(|_| "<LogEmitter>"),
            )
            .finish()
    }
}

impl ExecutionContext {
    /// Create a new execution context
    pub fn new(
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        branch: impl Into<String>,
        actor: impl Into<String>,
    ) -> Self {
        Self {
            execution_id: nanoid::nanoid!(),
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            branch: branch.into(),
            workspace_id: None,
            actor: actor.into(),
            trigger_name: None,
            event_data: None,
            http_request: None,
            input: serde_json::json!({}),
            metadata: HashMap::new(),
            started_at: Utc::now(),
            auth_context: None,
            allows_admin_escalation: false,
            log_emitter: None,
        }
    }

    /// Set workspace
    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace_id = Some(workspace.into());
        self
    }

    /// Set trigger name
    pub fn with_trigger(mut self, name: impl Into<String>) -> Self {
        self.trigger_name = Some(name.into());
        self
    }

    /// Set event data
    pub fn with_event_data(mut self, data: serde_json::Value) -> Self {
        self.event_data = Some(data);
        self
    }

    /// Set HTTP request
    pub fn with_http_request(mut self, request: HttpRequestData) -> Self {
        self.http_request = Some(request);
        self
    }

    /// Set input
    pub fn with_input(mut self, input: serde_json::Value) -> Self {
        self.input = input;
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Set authentication context for RLS filtering
    pub fn with_auth(mut self, auth: AuthContext) -> Self {
        self.auth_context = Some(auth);
        self
    }

    /// Allow admin escalation via raisin.asAdmin()
    pub fn with_admin_escalation(mut self, allowed: bool) -> Self {
        self.allows_admin_escalation = allowed;
        self
    }

    /// Set log emitter for real-time log streaming
    pub fn with_log_emitter(mut self, emitter: raisin_storage::LogEmitter) -> Self {
        self.log_emitter = Some(emitter);
        self
    }

    /// Get the node_id from event_data if available
    pub fn node_id(&self) -> Option<&str> {
        self.event_data
            .as_ref()
            .and_then(|e| e.get("node_id"))
            .and_then(|v| v.as_str())
    }

    /// Get the node_path from event_data if available
    pub fn node_path(&self) -> Option<&str> {
        self.event_data
            .as_ref()
            .and_then(|e| e.get("node_path"))
            .and_then(|v| v.as_str())
    }

    /// Get the node_type from event_data if available
    pub fn node_type(&self) -> Option<&str> {
        self.event_data
            .as_ref()
            .and_then(|e| e.get("node_type"))
            .and_then(|v| v.as_str())
    }

    /// Get the event_type from event_data if available
    pub fn event_type(&self) -> Option<&str> {
        self.event_data
            .as_ref()
            .and_then(|e| e.get("type"))
            .and_then(|v| v.as_str())
    }
}

/// HTTP request data for HTTP-triggered functions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequestData {
    /// HTTP method
    pub method: String,

    /// Request path (after /api/triggers/{name} or /api/webhooks/{id})
    pub path: String,

    /// Path parameters parsed from matchit route pattern
    /// e.g., for pattern "/:userId/orders/:orderId" and path "/123/orders/456"
    /// this would contain { "userId": "123", "orderId": "456" }
    #[serde(default)]
    pub path_params: HashMap<String, String>,

    /// Query parameters
    #[serde(default)]
    pub query_params: HashMap<String, String>,

    /// Request headers
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Request body (parsed as JSON if possible)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

/// Result of function execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Execution ID (matches ExecutionContext)
    pub execution_id: String,

    /// Whether execution succeeded
    pub success: bool,

    /// Return value from function
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,

    /// HTTP response (for HTTP-triggered functions that want custom responses)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_response: Option<HttpResponseData>,

    /// Execution statistics
    pub stats: ExecutionStats,

    /// Error information (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ExecutionError>,

    /// Logs captured during execution
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub logs: Vec<LogEntry>,

    /// When execution completed
    pub completed_at: DateTime<Utc>,
}

impl ExecutionResult {
    /// Create a successful result
    pub fn success(
        execution_id: impl Into<String>,
        output: serde_json::Value,
        stats: ExecutionStats,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            success: true,
            output: Some(output),
            http_response: None,
            stats,
            error: None,
            logs: Vec::new(),
            completed_at: Utc::now(),
        }
    }

    /// Create a failed result
    pub fn failure(
        execution_id: impl Into<String>,
        error: ExecutionError,
        stats: ExecutionStats,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            success: false,
            output: None,
            http_response: None,
            stats,
            error: Some(error),
            logs: Vec::new(),
            completed_at: Utc::now(),
        }
    }

    /// Add logs
    pub fn with_logs(mut self, logs: Vec<LogEntry>) -> Self {
        self.logs = logs;
        self
    }

    /// Set HTTP response
    pub fn with_http_response(mut self, response: HttpResponseData) -> Self {
        self.http_response = Some(response);
        self
    }
}

/// HTTP response data for functions that return custom HTTP responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponseData {
    /// HTTP status code
    pub status_code: u16,

    /// Response headers
    #[serde(default)]
    pub headers: HashMap<String, String>,

    /// Response body
    pub body: serde_json::Value,
}

impl HttpResponseData {
    /// Create a JSON response
    pub fn json(status: u16, body: serde_json::Value) -> Self {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Self {
            status_code: status,
            headers,
            body,
        }
    }

    /// Create a success response (200 OK)
    pub fn ok(body: serde_json::Value) -> Self {
        Self::json(200, body)
    }

    /// Create an error response
    pub fn error(status: u16, message: impl Into<String>) -> Self {
        Self::json(status, serde_json::json!({ "error": message.into() }))
    }
}

/// Execution statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionStats {
    /// Execution duration in milliseconds
    pub duration_ms: u64,

    /// Peak memory usage in bytes
    pub memory_used_bytes: u64,

    /// Number of instructions executed (for QuickJS)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions_executed: Option<u64>,

    /// Number of HTTP requests made
    #[serde(default)]
    pub http_requests_made: u32,

    /// Number of node operations performed
    #[serde(default)]
    pub node_operations: u32,

    /// Number of SQL queries executed
    #[serde(default)]
    pub sql_queries: u32,
}

/// Execution error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionError {
    /// Error code (e.g., "TIMEOUT", "RUNTIME_ERROR", "SYNTAX_ERROR")
    pub code: String,

    /// Human-readable error message
    pub message: String,

    /// Stack trace (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<String>,

    /// Line number where error occurred (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,

    /// Column number where error occurred (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

impl ExecutionError {
    /// Create a new execution error
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            stack_trace: None,
            line: None,
            column: None,
        }
    }

    /// Timeout error
    pub fn timeout(duration_ms: u64) -> Self {
        Self::new(
            "TIMEOUT",
            format!("Function execution timed out after {}ms", duration_ms),
        )
    }

    /// Memory limit error
    pub fn memory_limit(limit_bytes: u64) -> Self {
        Self::new(
            "MEMORY_LIMIT",
            format!("Function exceeded memory limit of {} bytes", limit_bytes),
        )
    }

    /// Syntax error
    pub fn syntax(message: impl Into<String>) -> Self {
        Self::new("SYNTAX_ERROR", message)
    }

    /// Runtime error
    pub fn runtime(message: impl Into<String>) -> Self {
        Self::new("RUNTIME_ERROR", message)
    }

    /// Network error
    pub fn network(message: impl Into<String>) -> Self {
        Self::new("NETWORK_ERROR", message)
    }

    /// URL not allowed error
    pub fn url_not_allowed(url: impl Into<String>) -> Self {
        Self::new(
            "URL_NOT_ALLOWED",
            format!("URL not in allowlist: {}", url.into()),
        )
    }

    /// With stack trace
    pub fn with_stack_trace(mut self, trace: impl Into<String>) -> Self {
        self.stack_trace = Some(trace.into());
        self
    }

    /// With location
    pub fn with_location(mut self, line: u32, column: u32) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ExecutionError {}

/// Log entry captured during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Log level
    pub level: LogLevel,

    /// Log message
    pub message: String,

    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

impl LogEntry {
    /// Create a new log entry
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
            timestamp: Utc::now(),
        }
    }

    pub fn debug(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Debug, message)
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Info, message)
    }

    pub fn warn(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Warn, message)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Error, message)
    }
}

/// Log levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Debug => write!(f, "debug"),
            Self::Info => write!(f, "info"),
            Self::Warn => write!(f, "warn"),
            Self::Error => write!(f, "error"),
        }
    }
}
