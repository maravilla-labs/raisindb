# Virtual Nodes Framework for RaisinDB

## Overview

Virtual Nodes is a **function-based framework** that allows users to integrate external systems — from cloud storage (Google Drive, S3) to IoT devices (Philips Hue, Nuki) to event streams (emails, webhooks) — into any workspace at any path. The framework is designed for an **app store model** where users can implement and share their own adapters.

**Core Principles:**
1. **Everything is a function** - All adapters are raisin-functions (JavaScript/Starlark)
2. **Everything is a node** - All configuration stored as nodes for admin-console management
3. **Mount anywhere** - Any workspace, any subtree, any depth
4. **User-extensible** - Custom adapters, custom mappings, custom triggers

## Configuration Architecture

### Two-Level Configuration

```
┌────────────────────────────────────────────────────────────────────┐
│                         TENANT LEVEL                                │
│  (shared across all databases/repos in tenant)                     │
│                                                                     │
│  /system/integrations/                                              │
│    /google-drive/          <- raisin:Integration node              │
│      - oauth_client_id                                              │
│      - oauth_client_secret (encrypted)                              │
│      - enabled: true                                                │
│      - connected_accounts: [...]                                    │
│    /bexio/                                                          │
│    /onedrive/                                                       │
└────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌────────────────────────────────────────────────────────────────────┐
│                       DATABASE LEVEL                                │
│  (per repository - mount configurations)                           │
│                                                                     │
│  /system/mounts/                                                    │
│    /team-drive/            <- raisin:VirtualMount node             │
│      - integration_ref: "/system/integrations/google-drive"        │
│      - account_ref: "account-123"                                   │
│      - target_workspace: "content"                                  │
│      - mount_path: "/documents/shared"                              │
│      - adapter_function: "/functions/adapters/google-drive"        │
│      - mapping_function: "/functions/mappers/google-docs"          │
│      - sync_config: { ... }                                         │
│      - enabled: true                                                │
│    /customer-data/                                                  │
│      - integration_ref: "/system/integrations/bexio"               │
│      - mount_path: "/crm/customers"                                 │
│      - ...                                                          │
└────────────────────────────────────────────────────────────────────┘
```

### Node Types

```yaml
# Tenant-level integration configuration
raisin:Integration:
  properties:
    - name: provider_type      # google-drive, bexio, onedrive, custom
    - name: oauth_config       # OAuth settings (client_id, etc.)
    - name: api_config         # API keys, endpoints
    - name: enabled            # boolean
    - name: connected_accounts # array of account references
    - name: adapter_function   # path to default adapter function

# Database-level mount configuration
raisin:VirtualMount:
  properties:
    - name: integration_ref    # reference to tenant Integration node
    - name: account_ref        # which connected account to use
    - name: target_workspace   # where to mount
    - name: mount_path         # path within workspace (any depth)
    - name: remote_root        # root folder/path in external system
    - name: adapter_function   # override adapter (optional)
    - name: mapping_function   # custom node type mapping (optional)
    - name: sync_config        # mode, interval, filters
    - name: write_config       # permissions, conflict handling
    - name: cache_config       # TTL, size limits
    - name: enabled            # boolean
```

---

## Function-Based Adapter Framework

### Adapter Function Interface

All adapters are **raisin-functions** implementing a standard interface:

```javascript
// /functions/adapters/google-drive/index.js
// Node type: raisin:Function

export async function handler(event, context) {
    const { operation, params } = event;
    const { credential, config } = context.metadata;

    switch (operation) {
        case "list":        return await listItems(credential, params);
        case "get":         return await getItem(credential, params);
        case "get_content": return await getContent(credential, params);
        case "create":      return await createItem(credential, params);
        case "update":      return await updateItem(credential, params);
        case "delete":      return await deleteItem(credential, params);
        case "get_changes": return await getChanges(credential, params);
        case "capabilities": return getCapabilities();
        default:
            throw new Error(`Unknown operation: ${operation}`);
    }
}

function getCapabilities() {
    return {
        can_read: true,
        can_write: true,
        can_create_folders: true,
        supports_changes: true,  // Delta sync
        supports_webhooks: true, // Push notifications
        supports_search: true,
        max_file_size: 5 * 1024 * 1024 * 1024, // 5GB
    };
}
```

### Custom Mapping Function

Users can define how external items map to RaisinDB node types:

```javascript
// /functions/mappers/google-docs/index.js
// Node type: raisin:Function

export async function handler(event, context) {
    const { external_item, mount_config } = event;

    // Default folder mapping
    if (external_item.is_folder) {
        return {
            node_type: "raisin:Folder",
            properties: {
                title: external_item.name,
            }
        };
    }

    // Custom mapping for Google Docs
    if (external_item.mime_type === "application/vnd.google-apps.document") {
        return {
            node_type: "myapp:GoogleDoc",
            properties: {
                title: external_item.name,
                googleDocId: external_item.external_id,
                lastEditor: external_item.metadata.lastModifyingUser,
                webUrl: external_item.web_url,
            }
        };
    }

    // Custom mapping for spreadsheets
    if (external_item.mime_type === "application/vnd.google-apps.spreadsheet") {
        return {
            node_type: "myapp:Spreadsheet",
            properties: {
                title: external_item.name,
                sheetId: external_item.external_id,
            }
        };
    }

    // Default file mapping
    return {
        node_type: "raisin:Asset",
        properties: {
            title: external_item.name,
            mimeType: external_item.mime_type,
            size: external_item.size_bytes,
        }
    };
}
```

---

## OAuth Flow Implementation

### OAuth Configuration (Tenant Level)

```javascript
// Stored in /system/integrations/google-drive
{
    "node_type": "raisin:Integration",
    "properties": {
        "provider_type": "google-drive",
        "oauth_config": {
            "client_id": "123456789.apps.googleusercontent.com",
            "client_secret_encrypted": "vault:google-oauth-secret",
            "auth_url": "https://accounts.google.com/o/oauth2/v2/auth",
            "token_url": "https://oauth2.googleapis.com/token",
            "scopes": [
                "https://www.googleapis.com/auth/drive.readonly",
                "https://www.googleapis.com/auth/drive.file"
            ],
            "redirect_uri": "https://app.raisindb.com/oauth/callback"
        },
        "enabled": true,
        "connected_accounts": []
    }
}
```

### OAuth Flow Sequence

```
┌──────────┐     ┌───────────────┐     ┌─────────────────┐     ┌──────────┐
│  Admin   │     │  Admin Console│     │   RaisinDB API  │     │  Google  │
│  Console │     │   Frontend    │     │                 │     │  OAuth   │
└────┬─────┘     └───────┬───────┘     └────────┬────────┘     └────┬─────┘
     │                   │                      │                    │
     │  Click "Connect   │                      │                    │
     │  Google Drive"    │                      │                    │
     │──────────────────>│                      │                    │
     │                   │                      │                    │
     │                   │  POST /oauth/start   │                    │
     │                   │  { provider: "google-drive" }             │
     │                   │─────────────────────>│                    │
     │                   │                      │                    │
     │                   │  { auth_url, state } │                    │
     │                   │<─────────────────────│                    │
     │                   │                      │                    │
     │                   │  Redirect to Google  │                    │
     │                   │─────────────────────────────────────────>│
     │                   │                      │                    │
     │                   │                      │   User grants      │
     │                   │                      │   permission       │
     │                   │                      │                    │
     │                   │  Redirect with code  │                    │
     │<──────────────────────────────────────────────────────────────│
     │                   │                      │                    │
     │  POST /oauth/callback                    │                    │
     │  { code, state }  │                      │                    │
     │──────────────────────────────────────────>│                    │
     │                   │                      │                    │
     │                   │                      │  Exchange code     │
     │                   │                      │  for tokens        │
     │                   │                      │───────────────────>│
     │                   │                      │                    │
     │                   │                      │  { access_token,   │
     │                   │                      │    refresh_token } │
     │                   │                      │<───────────────────│
     │                   │                      │                    │
     │                   │                      │  Store encrypted   │
     │                   │                      │  in tenant vault   │
     │                   │                      │                    │
     │                   │                      │  Update Integration│
     │                   │                      │  node with account │
     │                   │                      │                    │
     │  { success, account_id }                 │                    │
     │<─────────────────────────────────────────│                    │
```

### Token Refresh (Background Job)

```javascript
// Job: VirtualOAuthRefresh
// Scheduled trigger: Every 30 minutes

export async function handler(event, context) {
    // 1. Query all Integration nodes with OAuth
    const integrations = await raisin.sql.query(`
        SELECT * FROM nodes
        WHERE workspace = 'system'
        AND path LIKE '/system/integrations/%'
        AND node_type = 'raisin:Integration'
        AND properties->>'oauth_config' IS NOT NULL
    `);

    for (const integration of integrations) {
        for (const account of integration.properties.connected_accounts) {
            // 2. Check if token expires within 1 hour
            if (account.expires_at < Date.now() + 3600000) {
                // 3. Refresh token
                const newTokens = await refreshOAuthToken(
                    integration.properties.oauth_config,
                    account.refresh_token
                );

                // 4. Update account with new tokens
                await raisin.nodes.update(
                    "system",
                    integration.path,
                    {
                        properties: {
                            ...integration.properties,
                            connected_accounts: integration.properties.connected_accounts.map(a =>
                                a.id === account.id ? { ...a, ...newTokens } : a
                            )
                        }
                    }
                );
            }
        }
    }
}
```

---

## Trigger-Based Sync

### Sync Triggers

Virtual mounts use the existing trigger system.

> **Note:** RaisinDB already provides `/api/webhooks/{repo}/{id}` endpoints with nanoid-based secure URLs, so webhook-driven sync can leverage the existing infrastructure without new route definitions.

```javascript
// Stored in mount node's triggers property
{
    "triggers": [
        {
            "type": "schedule",
            "cron": "*/5 * * * *",  // Every 5 minutes
            "function": "/functions/sync/virtual-mount-sync"
        },
        {
            "type": "http",
            "path": "/webhooks/google-drive/{mount_id}",
            "method": "POST",
            "function": "/functions/sync/google-drive-webhook"
        }
    ]
}
```

### Sync Function

```javascript
// /functions/sync/virtual-mount-sync/index.js

export async function handler(event, context) {
    const { mount_id } = event;

    // 1. Load mount configuration
    const mount = await raisin.nodes.get("system", `/system/mounts/${mount_id}`);
    if (!mount || !mount.properties.enabled) return;

    // 2. Load adapter function
    const adapterPath = mount.properties.adapter_function;

    // 3. Get credential from integration
    const integration = await raisin.nodes.get("system", mount.properties.integration_ref);
    const account = integration.properties.connected_accounts.find(
        a => a.id === mount.properties.account_ref
    );

    // 4. Call adapter to get changes
    const changes = await raisin.functions.execute(adapterPath, {
        operation: "get_changes",
        params: {
            since_token: mount.properties.last_sync_token,
            folder_id: mount.properties.remote_root
        },
        credential: account
    });

    // 5. Process changes
    for (const change of changes.items) {
        await processChange(mount, change);
    }

    // 6. Update sync state
    await raisin.nodes.update("system", mount.path, {
        properties: {
            ...mount.properties,
            last_sync_token: changes.next_token,
            last_sync_at: new Date().toISOString()
        }
    });
}

async function processChange(mount, change) {
    const fullPath = `${mount.properties.mount_path}${change.relative_path}`;

    switch (change.type) {
        case "created":
        case "updated":
            // Map external item to node
            const mapped = await raisin.functions.execute(
                mount.properties.mapping_function || "/functions/mappers/default",
                { external_item: change.item, mount_config: mount.properties }
            );

            // Create/update node in target workspace
            await raisin.nodes.upsert(
                mount.properties.target_workspace,
                fullPath,
                {
                    node_type: mapped.node_type,
                    properties: {
                        ...mapped.properties,
                        __virtual: true,
                        __mount_id: mount.id,
                        __external_id: change.item.external_id,
                        __etag: change.item.etag,
                    }
                }
            );
            break;

        case "deleted":
            await raisin.nodes.delete(
                mount.properties.target_workspace,
                fullPath
            );
            break;
    }
}
```

---

## Event-Driven Integrations

Virtual Nodes aren't limited to mounting external storage. The same framework supports **event-driven and transient integration patterns**:

- **IoT device integrations** — Philips Hue lights, Nuki smart locks, temperature sensors
- **Message/email processing** — ephemeral nodes for incoming messages that trigger workflows
- **Webhook-driven events** — external systems push state changes that become actionable nodes

The key insight: **nodes can be transient event carriers**, not just persistent data. The existing trigger system (`type: "http"`, `type: "schedule"`, `type: "node_event"`) already supports these patterns.

### Pattern Comparison

| Pattern | Node Lifecycle | Example |
|---------|---------------|---------|
| Storage Mount | Persistent, synced | Google Drive files |
| Device State | Persistent, updated | Philips Hue light status |
| Event Stream | Ephemeral, processed & deleted | Incoming emails, webhooks |

### IoT Device Example: Philips Hue

Smart home devices can be represented as virtual nodes that reflect their current state:

```yaml
# VirtualMount for Philips Hue
raisin:VirtualMount:
  integration_ref: "/system/integrations/philips-hue"
  target_workspace: "devices"
  mount_path: "/lights"
  adapter_function: "/functions/adapters/philips-hue"
  sync_config:
    mode: "poll"
    interval_seconds: 30
```

The adapter returns device state, materialized as nodes like `/devices/lights/living-room`:

```javascript
{
    node_type: "iot:Light",
    properties: {
        name: "Living Room",
        on: true,
        brightness: 80,
        color: "#FFE4B5",
        __virtual: true,
        __external_id: "hue-light-1"
    }
}
```

**Trigger on state change:**

```javascript
{
    "type": "node_event",
    "events": ["updated"],
    "filter": { "node_type": "iot:Light" },
    "function": "/functions/automation/light-changed"
}
```

### Ephemeral Event Example: Email Processing

Incoming emails become transient nodes that trigger processing workflows:

```yaml
# VirtualMount for email inbox
raisin:VirtualMount:
  integration_ref: "/system/integrations/email-imap"
  target_workspace: "inbox"
  mount_path: "/messages"
  adapter_function: "/functions/adapters/imap"
  sync_config:
    mode: "poll"
    interval_seconds: 60
    ephemeral: true        # Mark nodes for cleanup after processing
    ttl_seconds: 3600      # Auto-delete after 1 hour if not processed
```

**Workflow:**
1. Sync adapter polls inbox → creates `/inbox/messages/msg-{id}` nodes
2. `on_create` trigger fires → processing function runs (extract data, create ticket, etc.)
3. Function deletes the node (or moves to `/archive/`) after processing

### Webhook-Driven Events

External systems can push events directly via webhooks, creating ephemeral nodes:

```
External System (Nuki) → POST /api/webhooks/{repo}/{id}
                              ↓
                     Webhook handler creates node
                              ↓
                     /events/door/unlock-{timestamp}
                              ↓
                     on_create trigger fires
                              ↓
                     Function processes event
                              ↓
                     Node deleted (or retained for audit)
```

### Extended Adapter Capabilities

Event-driven adapters can declare additional capabilities:

```javascript
function getCapabilities() {
    return {
        // Existing capabilities
        can_read: true,
        supports_changes: true,
        supports_webhooks: true,

        // Event-driven capabilities
        supports_push: true,          // Can receive push events
        default_ttl: 3600,            // Suggested TTL for ephemeral nodes
        event_types: ["state_change", "alert", "notification"]
    };
}
```

### Design Note

The same infrastructure powers all patterns:
- Same `VirtualMount` node type
- Same adapter function interface
- Same trigger system
- Same caching layer (just with shorter TTL for ephemeral nodes)

The only difference is **intent**: persistent data vs. transient events.

---

## Path Resolution

Mount anywhere capability requires path-aware interception:

```
User requests: GET /content/crm/customers/contact-123

1. NodeService.get_by_path("/crm/customers/contact-123")
       │
       ▼
2. VirtualPathResolver.resolve(workspace="content", path="/crm/customers/contact-123")
       │
       ├── Query: SELECT * FROM nodes WHERE workspace='system'
       │          AND node_type='raisin:VirtualMount'
       │          AND properties->>'target_workspace' = 'content'
       │
       ▼
3. Found mount: target_workspace="content", mount_path="/crm/customers"
       │
       ▼
4. Extract relative_path: "/contact-123"
       │
       ▼
5. Check cache for virtual node
       │
       ├── Cache hit → Return cached node
       │
       └── Cache miss →
           │
           ▼
6. Invoke adapter function:
   raisin.functions.execute(mount.adapter_function, {
       operation: "get",
       params: { path: "/contact-123" },
       credential: account
   })
       │
       ▼
7. Map result via mapping function → Store in cache → Return node
```

---

## Caching Layer

### Hybrid Cache Strategy

Virtual nodes are cached as **regular nodes** with virtual metadata properties:

```javascript
// Virtual node stored in target workspace
{
    "id": "virtual:gdrive-abc123",
    "name": "Q4 Report.docx",
    "path": "/documents/shared/Q4 Report.docx",
    "node_type": "raisin:Asset",
    "workspace": "content",
    "properties": {
        "title": "Q4 Report.docx",
        "mimeType": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "size": 245678,
        // Virtual metadata (for sync and cache management)
        "__virtual": true,
        "__mount_id": "mount-team-drive",
        "__external_id": "1ABC...xyz",
        "__etag": "abc123def456",
        "__cached_at": "2024-01-15T10:30:00Z",
        "__cache_ttl": 300  // seconds
    }
}
```

### Cache Invalidation

**Invalidation Strategies:**

1. **Background Sync (Primary):**
   - Scheduled job runs every N minutes (configurable per mount)
   - Uses adapter's `get_changes` operation (delta sync)
   - Updates `last_sync_token` for efficient incremental sync

2. **On-Access Refresh:**
   - When `__cached_at + __cache_ttl < now`, refresh before returning
   - Provides eventual consistency for stale-while-revalidate pattern

3. **Webhook Push (Optimal):**
   - External system pushes change notifications
   - HTTP trigger receives webhook, enqueues refresh job
   - Minimizes sync latency

```javascript
// On-demand refresh trigger
{
    "type": "node_event",
    "events": ["accessed"],
    "filter": {
        "properties": {
            "__virtual": true,
            "__cached_at": { "$lt": "now() - interval '5 minutes'" }
        }
    },
    "function": "/functions/sync/refresh-virtual-node"
}
```

**Write-Through Cache:**
- Creates/updates/deletes propagate to external system first
- On success, update local cached node
- On failure, return error (don't update cache)

---

## Google Drive Adapter Implementation

### Complete Adapter Function

```javascript
// /functions/adapters/google-drive/index.js

const DRIVE_API = "https://www.googleapis.com/drive/v3";

export async function handler(event, context) {
    const { operation, params } = event;
    const { credential } = context.metadata;

    // Ensure valid access token
    const accessToken = await ensureValidToken(credential);

    switch (operation) {
        case "list":
            return await listItems(accessToken, params);
        case "get":
            return await getItem(accessToken, params);
        case "get_content":
            return await getContent(accessToken, params);
        case "create":
            return await createItem(accessToken, params);
        case "update":
            return await updateItem(accessToken, params);
        case "delete":
            return await deleteItem(accessToken, params);
        case "get_changes":
            return await getChanges(accessToken, params);
        case "capabilities":
            return getCapabilities();
        default:
            throw new Error(`Unknown operation: ${operation}`);
    }
}

async function ensureValidToken(credential) {
    // Token refresh is handled by the OAuth refresh job
    // This function just validates the token exists
    if (!credential.access_token) {
        throw new Error("No access token available");
    }
    return credential.access_token;
}

async function listItems(token, { folder_id, cursor, limit = 100 }) {
    const query = folder_id
        ? `'${folder_id}' in parents and trashed = false`
        : `'root' in parents and trashed = false`;

    const params = new URLSearchParams({
        q: query,
        fields: "nextPageToken,files(id,name,mimeType,size,parents,createdTime,modifiedTime,md5Checksum,webViewLink)",
        pageSize: limit.toString(),
    });

    if (cursor) {
        params.set("pageToken", cursor);
    }

    const response = await raisin.http.fetch(
        `${DRIVE_API}/files?${params}`,
        {
            headers: { Authorization: `Bearer ${token}` }
        }
    );

    if (!response.ok) {
        throw new Error(`Drive API error: ${response.status}`);
    }

    const data = response.body;

    return {
        items: data.files.map(mapDriveFile),
        next_cursor: data.nextPageToken || null,
        total_count: null  // Drive doesn't provide total count
    };
}

async function getItem(token, { item_id, path }) {
    // If path provided, resolve to item_id first
    if (path && !item_id) {
        item_id = await resolvePathToId(token, path);
        if (!item_id) return null;
    }

    const fields = "id,name,mimeType,size,parents,createdTime,modifiedTime,md5Checksum,webViewLink,webContentLink";

    const response = await raisin.http.fetch(
        `${DRIVE_API}/files/${item_id}?fields=${fields}`,
        {
            headers: { Authorization: `Bearer ${token}` }
        }
    );

    if (response.status === 404) return null;
    if (!response.ok) throw new Error(`Drive API error: ${response.status}`);

    return mapDriveFile(response.body);
}

async function getContent(token, { item_id }) {
    const response = await raisin.http.fetch(
        `${DRIVE_API}/files/${item_id}?alt=media`,
        {
            headers: { Authorization: `Bearer ${token}` }
        }
    );

    if (!response.ok) throw new Error(`Drive API error: ${response.status}`);

    return {
        content: response.body,  // Binary content
        mime_type: response.headers["content-type"],
    };
}

async function getChanges(token, { since_token, folder_id }) {
    // Get start page token if we don't have one
    let pageToken = since_token;
    if (!pageToken) {
        const startResponse = await raisin.http.fetch(
            `${DRIVE_API}/changes/startPageToken`,
            { headers: { Authorization: `Bearer ${token}` } }
        );
        pageToken = startResponse.body.startPageToken;
    }

    const response = await raisin.http.fetch(
        `${DRIVE_API}/changes?pageToken=${pageToken}&fields=nextPageToken,newStartPageToken,changes(changeType,removed,file(id,name,mimeType,size,parents,createdTime,modifiedTime,md5Checksum))`,
        { headers: { Authorization: `Bearer ${token}` } }
    );

    const data = response.body;
    const changes = [];

    for (const change of data.changes || []) {
        if (change.file) {
            // Filter by folder_id if specified
            if (folder_id && !isDescendantOf(change.file, folder_id)) {
                continue;
            }

            changes.push({
                type: change.removed ? "deleted" : "updated",
                item: mapDriveFile(change.file),
                relative_path: await getRelativePath(token, change.file, folder_id)
            });
        }
    }

    return {
        items: changes,
        next_token: data.newStartPageToken || data.nextPageToken
    };
}

function mapDriveFile(file) {
    return {
        external_id: file.id,
        name: file.name,
        mime_type: file.mimeType,
        size_bytes: file.size ? parseInt(file.size) : null,
        is_folder: file.mimeType === "application/vnd.google-apps.folder",
        parent_id: file.parents?.[0] || null,
        created_at: file.createdTime,
        modified_at: file.modifiedTime,
        etag: file.md5Checksum,
        web_url: file.webViewLink,
        download_url: file.webContentLink,
        metadata: {
            driveId: file.id,
            mimeType: file.mimeType
        }
    };
}

async function resolvePathToId(token, path) {
    const parts = path.split("/").filter(p => p);
    let currentId = "root";

    for (const part of parts) {
        const query = `name = '${part}' and '${currentId}' in parents and trashed = false`;
        const response = await raisin.http.fetch(
            `${DRIVE_API}/files?q=${encodeURIComponent(query)}&fields=files(id)`,
            { headers: { Authorization: `Bearer ${token}` } }
        );

        const files = response.body.files;
        if (!files || files.length === 0) return null;
        currentId = files[0].id;
    }

    return currentId;
}

function getCapabilities() {
    return {
        can_read: true,
        can_write: true,
        can_create_folders: true,
        supports_changes: true,
        supports_webhooks: true,
        supports_search: true,
        max_file_size: 5 * 1024 * 1024 * 1024
    };
}
```

---

## Admin Console Integration

Since everything is a node, the admin-console can manage virtual nodes configuration:

### Integration Management Page

```typescript
// Admin Console: Integrations page
// Path: /settings/integrations

// List all integrations (tenant level)
const integrations = await api.query(`
    SELECT * FROM nodes
    WHERE workspace = 'system'
    AND path LIKE '/system/integrations/%'
    AND node_type = 'raisin:Integration'
`);

// Create new integration
await api.nodes.create("system", "/system/integrations", {
    name: "google-drive",
    node_type: "raisin:Integration",
    properties: {
        provider_type: "google-drive",
        oauth_config: { ... },
        enabled: true
    }
});
```

### Mount Management Page

```typescript
// Admin Console: Virtual Mounts page
// Path: /settings/mounts (database level)

// List all mounts for current database
const mounts = await api.query(`
    SELECT * FROM nodes
    WHERE workspace = 'system'
    AND path LIKE '/system/mounts/%'
    AND node_type = 'raisin:VirtualMount'
`);

// Create new mount
await api.nodes.create("system", "/system/mounts", {
    name: "team-drive",
    node_type: "raisin:VirtualMount",
    properties: {
        integration_ref: "/system/integrations/google-drive",
        account_ref: "account-123",
        target_workspace: "content",
        mount_path: "/documents/shared",
        adapter_function: "/functions/adapters/google-drive",
        sync_config: {
            mode: "hybrid",
            interval_seconds: 300,
            exclude_patterns: ["*.tmp", "~$*"]
        },
        enabled: true
    }
});
```

---

## Extensibility: Package-Based Adapter Model

Adapters are distributed as `.rap` packages through the existing package system. This enables an app-store model where users install, configure, and manage integrations without touching Rust code.

### Where Integrations Live

| Workspace | Path | Content |
|-----------|------|---------|
| `packages` | `/packages/google-drive-adapter/` | Installed `.rap` package metadata |
| `system` | `/system/integrations/google-drive/` | `raisin:Integration` config node (OAuth, enabled flag) |
| `functions` | `/functions/adapters/google-drive/` | Adapter function (deployed from package) |
| `functions` | `/functions/mappers/google-drive-default/` | Mapping function (deployed from package) |

### Creating a Custom Adapter

A `.rap` package for an integration follows this structure:

```
my-adapter/
├── manifest.yaml
└── content/
    └── functions/
        ├── adapters/
        │   └── my-adapter/
        │       └── index.js       # Implements the adapter interface
        └── mappers/
            └── my-adapter-default/
                └── index.js       # Default type mapping
```

```yaml
# manifest.yaml
name: my-adapter
version: 1.0.0
category: integrations          # Marks this as a virtual-node adapter
description: My custom external storage adapter
provides:
  functions:
    - adapters/my-adapter
    - mappers/my-adapter-default
```

### Installation Flow

1. User uploads `.rap` file (or selects from a catalog)
2. `PackageInstall` job extracts the package and deploys functions into the `functions` workspace
3. Admin console discovers the new adapter (queries packages with `category: integrations`)
4. Admin creates an `raisin:Integration` node in `/system/integrations/` with provider credentials
5. Admin creates a `raisin:VirtualMount` node referencing the integration and adapter function

### Self-Configuring Packages

Packages can ship pre-configured nodes in their `content/` directory:

```
my-adapter/
├── manifest.yaml
└── content/
    ├── functions/
    │   └── adapters/my-adapter/index.js
    └── system/
        └── integrations/
            └── my-adapter/       # Pre-configured Integration node template
                └── node.json     # Defaults for oauth_config, scopes, etc.
```

On install, the package system materializes these nodes into the appropriate workspaces, giving users a working starting point that they customize through the admin console.

### Discovery Contract

The admin console discovers available adapters by:

1. **Querying installed packages:** `SELECT * FROM 'packages' WHERE properties->>'category'::String = 'integrations'`
2. **Reading `provides.functions`** from the package manifest to know which adapter/mapper functions are available
3. **Querying active integrations:** `SELECT * FROM 'system' WHERE node_type = 'raisin:Integration'`
4. **Querying mounts:** `SELECT * FROM 'system' WHERE node_type = 'raisin:VirtualMount'`

---

## Implementation Roadmap

### Phase 1: Node Types & Configuration
1. Create `crates/raisin-core/global_nodetypes/raisin_integration.yaml`
2. Create `crates/raisin-core/global_nodetypes/raisin_virtual_mount.yaml`
3. Update `crates/raisin-core/global_workspaces/system.yaml` with new types
4. Add `VirtualMountSync` and `OAuthTokenRefresh` variants to `JobType` enum
5. Add initial structure for `/system/integrations/` and `/system/mounts/`

### Phase 2: OAuth Implementation
1. Create `crates/raisin-server/src/routes/oauth.rs`
2. Implement `/api/oauth/start`, `/api/oauth/callback`, `/api/oauth/disconnect`
3. Store tokens encrypted in Integration node's `connected_accounts`
4. Add `OAuthTokenRefresh` job handler using unified job queue (`JobRegistry.register_job()` + `JobDataStore.put()`)

### Phase 3: Sync Engine
1. Create job handler at `crates/raisin-rocksdb/src/jobs/handlers/virtual_mount_sync/`
2. Handler reads `VirtualMount` node config, resolves credentials from `Integration` node
3. Invokes adapter function via `raisin.functions.execute()` with `get_changes` operation
4. Materializes returned items as regular nodes in the target workspace (with `__virtual` metadata)
5. Updates `last_sync_token` and `last_sync_at` on the mount node
6. Cached virtual nodes are regular RocksDB rows — SQL queries work transparently (see [SQL Integration](#sql-integration))

### Phase 4: Google Drive Adapter (Builtin Package)
1. Create `builtin-packages/google-drive-adapter/manifest.yaml` with `category: integrations`
2. Implement adapter function: list, get, get_content, create, update, delete, get_changes
3. Create default mapping function for Docs, Sheets, Slides, and generic files
4. Package ships as a `.rap` file installed during server bootstrap

### Phase 5: Admin Console Integration
1. Add Integrations management page at `/settings/integrations`
2. Add OAuth connection flow UI with popup/redirect handling
3. Add Mount configuration page at `/settings/mounts`
4. Add sync status display and manual sync button

### Phase 6: On-Demand Resolution (Optional)
1. Add on-access refresh path for stale cached virtual nodes
2. VirtualPathResolver intercepts `get_by_path()` for cache-miss cases
3. Falls back to adapter invocation when node not yet materialized
4. This is an optimization over Phase 3's background-only sync

---

## Key Files to Create/Modify

### New Node Type Definitions (YAML)
| File | Purpose |
|------|---------|
| `crates/raisin-core/global_nodetypes/raisin_integration.yaml` | Integration node type definition |
| `crates/raisin-core/global_nodetypes/raisin_virtual_mount.yaml` | VirtualMount node type definition |
| `crates/raisin-core/global_workspaces/system.yaml` | Add new types to allowed_node_types |

### Sync Engine (Rust)
| File | Purpose |
|------|---------|
| `crates/raisin-rocksdb/src/jobs/handlers/virtual_mount_sync/mod.rs` | Sync job handler entry point (NEW) |
| `crates/raisin-rocksdb/src/jobs/handlers/virtual_mount_sync/adapter.rs` | Adapter invocation logic (NEW) |
| `crates/raisin-rocksdb/src/jobs/handlers/virtual_mount_sync/materializer.rs` | Node materialization into target workspace (NEW) |

### OAuth Support (Rust)
| File | Purpose |
|------|---------|
| `crates/raisin-server/src/routes/oauth.rs` | OAuth flow endpoints (NEW) |
| `crates/raisin-server/src/routes/mod.rs` | Add oauth module |
| `crates/raisin-rocksdb/src/jobs/handlers/oauth_refresh.rs` | Token refresh job handler (NEW) |

### Google Drive Adapter Package
| File | Purpose |
|------|---------|
| `builtin-packages/google-drive-adapter/manifest.yaml` | Package manifest with `category: integrations` |
| `builtin-packages/google-drive-adapter/content/functions/adapters/google-drive/index.js` | Google Drive adapter function |
| `builtin-packages/google-drive-adapter/content/functions/mappers/google-drive-default/index.js` | Default node type mapper |

### Function API (Already Exists)
- `raisin.functions.execute()` - Function-to-function calls (in `crates/raisin-functions/src/api/mod.rs`)

---

## SQL Integration

Virtual nodes are cached as **regular nodes** in the target workspace (see [Caching Layer](#caching-layer)). Because the sync engine materializes them into RocksDB with standard node keys, all SQL queries work transparently — no storage-layer interception or SQL engine modifications are needed.

```sql
-- These queries just work; virtual nodes are regular rows
SELECT * FROM 'content' WHERE path LIKE '/documents/shared/%'
SELECT * FROM 'content' WHERE properties->>'__virtual'::String = 'true'
```

The `__virtual`, `__mount_id`, and `__external_id` metadata properties are available for filtering but otherwise virtual nodes are indistinguishable from nodes created directly.

---

## Design Decisions

| Question | Decision | Implication |
|----------|----------|-------------|
| **Node lifecycle** | Configurable: persistent or ephemeral | `sync_config.ephemeral` + `ttl_seconds` for transient event patterns |
| **Versioning** | No versioning | Virtual nodes are always "live" from external source |
| **Relations** | Full support | Relations can link local ↔ virtual nodes |
| **Provider priority** | Google Drive first | Start with Google Drive OAuth2 adapter |
| **Adapter approach** | All function-based | Enables app store model, user extensibility |
| **Configuration scope** | Repository level | Simpler, fits existing architecture. Each repo manages its own integrations |
| **SQL integration** | Cached as regular nodes — transparent | No storage-layer or SQL engine modifications needed |
| **Initial scope** | Full CRUD | Complete read/write sync from start |
| **Function invocation** | Use `raisin.functions.execute()` | Existing API in codebase (not `invoke()`) |
| **Extensibility model** | Package-based (`.rap`) — `category: integrations` | Adapters ship as installable packages via the package system |
| **On-demand resolution** | Deferred to Phase 6 | v1 uses background sync only; on-access resolution is an optimization |

---

## Coding Standards

Implementation should follow these project conventions:

- **DRY** — extract shared logic into reusable helpers; avoid copy-paste across job handlers or adapter functions
- **300-line file maximum** — split larger modules into sub-modules (e.g., `virtual_mount_sync/mod.rs`, `adapter.rs`, `materializer.rs`)
- **Modular directory structure** — one concern per file; group related files under a directory with `mod.rs`
- **Traits for testability** — define trait boundaries (e.g., `AdapterInvoker`, `NodeMaterializer`) so sync logic can be tested without real storage or network calls
- **`raisin-error` types with `thiserror`** — all new error variants go through the existing error crate; use `anyhow` at boundaries only
