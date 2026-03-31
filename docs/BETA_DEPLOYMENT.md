# RaisinDB Beta Deployment: Brainstorming

## Your Setup
- **Server:** Hetzner AX42 -- 6 cores / 12 threads, 64 GB RAM, 2x512 GB NVMe
- **Domain:** `maravilla.cloud` (subdomains: `acme.maravilla.cloud`, `myproject.maravilla.cloud`, etc.)
- **Goal:** Host beta customers + your own projects on one box

---

## The Two Options (and why multi-tenant wins)

### Option A: Firecracker VMs (one RaisinDB per customer)

Each customer gets an isolated microVM running their own RaisinDB process.

```
+-----------------------------------------------------+
|  Hetzner AX42 (64 GB RAM)                           |
|                                                     |
|  +----------+ +----------+ +----------+            |
|  | VM: acme | | VM: beta | | VM: yours|  ...       |
|  | RaisinDB | | RaisinDB | | RaisinDB |            |
|  | ~1.8 GB  | | ~1.8 GB  | | ~1.8 GB  |            |
|  +----------+ +----------+ +----------+            |
|                                                     |
|  Orchestrator (VM lifecycle, networking, routing)   |
+-----------------------------------------------------+
```

**Per-instance memory breakdown:**
- RocksDB block cache + write buffers: ~500 MB
- Tantivy fulltext index cache: ~512 MB
- HNSW vector index cache: ~512 MB
- Job system + tokio runtime: ~175 MB
- **Total: ~1.76 GB per instance**

**On 64 GB RAM (with ~50 GB usable after OS):**
- Max ~28 instances (if all idle with minimal data)
- Realistically: **15-20 customers** (with data growth headroom)

**Pros:**
- Perfect isolation (security, crashes, noisy neighbor)
- Can give customers root access to their VM
- Easy per-customer resource limits (cgroups)

**Cons:**
- 1.76 GB wasted per customer on duplicated caches
- Complex orchestration (Firecracker API, tap networking, health checks)
- Rolling updates = restart 20 VMs individually
- Overkill for beta with small data volumes

### Option B: Multi-Tenant Single Instance (recommended)

One RaisinDB process serves all customers. Isolation via key prefixing in storage.

```
+---------------------------------------------------------+
|  Hetzner AX42 (64 GB RAM)                               |
|                                                         |
|  +-----------------------------------------------------+|
|  |  RaisinDB (single process, ~2-4 GB total)           ||
|  |                                                     ||
|  |  Storage keys:                                      ||
|  |    acme\0repo1\0main\0ws\0nodes\0...                ||
|  |    beta\0repo1\0main\0ws\0nodes\0...                ||
|  |    myproj\0repo1\0main\0ws\0nodes\0...              ||
|  |                                                     ||
|  |  Shared: RocksDB, Tantivy, HNSW, Job Queue         ||
|  +-----------------------------------------------------+|
|                                                         |
|  Caddy (TLS termination, wildcard *.maravilla.cloud)   |
+---------------------------------------------------------+
```

**Memory usage:**
- One RocksDB instance: ~500 MB block cache + write buffers (shared across all tenants via key-prefix isolation)
- One Tantivy engine: 512 MB LRU cache shared across all tenants. Each tenant/repo/branch gets its own index files on disk, loaded on-demand into the cache. Inactive indexes are evicted (~30 MB per branch index).
- One HNSW engine: 512 MB LRU cache shared across all tenants. Each tenant/repo/branch gets its own vector index file on disk, loaded on-demand. Dirty indexes are persisted every 60s. Workspace filtering is applied at search time.
- Per-tenant memory overhead: **negligible** — just key prefixes in RocksDB and LRU cache entries for active branches
- **Total: ~2-4 GB fixed ceiling regardless of customer count** (cache sizes are capped, not per-tenant)

**On 64 GB RAM:**
- Effectively unlimited beta customers (data-bound, not instance-bound)
- Even with 50 customers x 1 GB data each = 50 GB, still fits in RAM

### Capacity Estimate: AMD Ryzen 5 3600 (6c/12t @ 3.6 GHz, 64 GB DDR4, 2x 512 GB NVMe)

| Resource | Budget | Per-tenant (typical beta user) | Estimated tenants |
|----------|--------|-------------------------------|-------------------|
| **RAM** | ~58 GB usable (64 GB minus ~4 GB OS, ~2-4 GB fixed caches) | ~50-200 MB hot working set | **200-500** |
| **Disk** | ~900 GB usable (2x NVMe, data + WAL split) | 1-5 GB (with RocksDB 1.5x amplification) | **120-600** |
| **CPU** | 12 threads | Mostly idle (beta = low QPS per tenant) | **100-200** concurrent active |
| **I/O** | ~6 GB/s sequential read (NVMe) | Bursty; cache misses hit SSD | Not a bottleneck at beta scale |

**Bottleneck analysis:**
- **Storage is the binding constraint.** At 5 GB per tenant (heavy user with vectors + full-text), 2x 512 GB NVMe gives ~120 tenants. At 1 GB per tenant (typical small app), ~600 tenants.
- **RAM is not a bottleneck.** RocksDB/Tantivy/HNSW caches are fixed-size (capped at ~2-4 GB total). Tenant data lives on disk; only hot blocks are cached. 64 GB gives enormous headroom for OS page cache.
- **CPU is not a bottleneck at beta scale.** 12 threads handle concurrent queries well. RaisinDB is I/O-bound, not CPU-bound. Heavy vector search or full-text indexing are the most CPU-intensive operations.

**Conservative safe target: 50-100 tenants**
- Assumes some tenants will have heavy vector indexes, full-text search, and active function triggers
- Leaves 50%+ headroom for traffic spikes and background jobs (compaction, index persistence, replication)

**Comfortable capacity: 100-200 tenants**
- Assumes typical beta users with small datasets (< 2 GB each)
- Low concurrent activity (< 5 active users per tenant simultaneously)

**Theoretical maximum: 300-500 tenants**
- Requires most tenants to be low-activity with small datasets (< 500 MB each)
- Noisy-neighbor risk increases without rate limiting enforced

**Recommendations:**
- Split NVMe drives: one for RocksDB data/WAL, one for vector indexes + Tantivy files + backups
- Monitor with `iostat` and RocksDB statistics — if P99 read latency exceeds 10ms, you're cache-thrashing
- Start accepting beta users in batches of 20-30, monitor resource usage before opening more slots

**Pros:**
- 90% of multi-tenancy already built (storage isolation, per-tenant auth, CORS)
- Only need: subdomain -> tenant_id resolution in middleware (~50 lines of Rust)
- Single binary to deploy and update
- Shared caches = better resource utilization
- 64 GB RAM is massive overkill for this approach (great headroom)

**Cons:**
- Noisy neighbor risk (one tenant's heavy query slows others) -- mitigated by rate limiting
- Single process failure affects all tenants -- mitigated by systemd auto-restart
- Need to implement TenantLimits enforcement (currently defined but not checked)

---

## What Already Exists in the Codebase

The multi-tenant infrastructure is ~90% complete:

| Feature | Status | Location |
|---------|--------|----------|
| Key-prefix isolation (`tenant\0repo\0...`) | Done | `crates/raisin-rocksdb/src/keys.rs` |
| `TenantInfo` in middleware | Done | `crates/raisin-transport-http/src/middleware.rs:232` |
| `X-Tenant-ID` header extraction | Done | `ensure_tenant_middleware` |
| Per-tenant auth config (providers, anonymous, CORS) | Done | `TenantAuthConfigRepository` |
| Tenant registry (create/list/delete) | Done | `crates/raisin-rocksdb/src/repositories/registry.rs` |
| Hierarchical CORS (repo -> tenant -> global) | Done | `unified_cors_middleware` |
| Per-tenant JWT tokens | Done | `tenant_id` in claims |
| `TenantLimits` struct | Defined, not enforced | `crates/raisin-rocksdb/src/config.rs` |
| `IsolationMode::Dedicated` | Defined, not used | `crates/raisin-storage/src/lib.rs` |
| **Subdomain -> tenant_id resolution** | **Missing** | Need ~50 lines in middleware |
| **Wildcard CORS matching** | **Missing** | Need pattern matching |
| **Rate limiting per tenant** | **Missing** | Post-beta |
| **Quota enforcement** | **Missing** | Post-beta |

---

## The Code Change (Small)

The only code needed for beta launch is modifying `ensure_tenant_middleware` to parse the `Host` header:

```rust
// Current (middleware.rs, ensure_tenant_middleware):
let tenant_id = req
    .headers()
    .get("x-tenant-id")
    .and_then(|v| v.to_str().ok())
    .unwrap_or("default")
    .to_string();

// New logic (conceptual):
let tenant_id = resolve_tenant_from_request(&req, &state.base_domain);

fn resolve_tenant_from_request(req: &Request, base_domain: &Option<String>) -> String {
    // 1. Try subdomain from Host header
    if let Some(base) = base_domain {
        let host = req.headers()
            .get("x-forwarded-host")  // Behind reverse proxy
            .or_else(|| req.headers().get("host"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        // Strip port: "acme.maravilla.cloud:443" -> "acme.maravilla.cloud"
        let host = host.split(':').next().unwrap_or(host);

        // Check if host ends with base domain and has a subdomain part
        if let Some(subdomain) = host.strip_suffix(&format!(".{}", base)) {
            if !subdomain.is_empty()
                && !["www", "api", "admin", "app"].contains(&subdomain)
            {
                return subdomain.to_string();
            }
        }
    }

    // 2. Fall back to X-Tenant-ID header
    req.headers()
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("default")
        .to_string()
}
```

Plus adding `base_domain: Option<String>` to config and AppState (3 files, ~5 lines each).

---

## Infrastructure Setup

### DNS (Cloudflare or any provider)
```
A    maravilla.cloud        -> 65.x.x.x  (your Hetzner IP)
A    *.maravilla.cloud      -> 65.x.x.x  (wildcard)
```

### Caddy (Reverse Proxy + Auto-TLS)
```
*.maravilla.cloud, maravilla.cloud {
    tls {
        dns cloudflare {env.CF_API_TOKEN}  # Wildcard cert via DNS-01 challenge
    }
    reverse_proxy 127.0.0.1:8080
}
```

Why Caddy:
- Automatic HTTPS cert issuance + renewal (including wildcards via DNS challenge)
- Zero-config compared to nginx + certbot
- ~10 MB binary, minimal resource usage

### systemd (`/etc/systemd/system/raisindb.service`)
```ini
[Unit]
Description=RaisinDB
After=network.target

[Service]
Type=simple
User=raisindb
ExecStart=/usr/local/bin/raisin-server --config /etc/raisindb/production.toml
Restart=always
RestartSec=3
Environment=RUST_LOG=info
Environment=RAISINDB_MASTER_KEY=<random-32-byte-hex>
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

### RaisinDB Config (`/etc/raisindb/production.toml`)
```toml
[server]
port = 8080
bind_address = "127.0.0.1"   # Only Caddy talks to us
data_dir = "/var/lib/raisindb/data"
base_domain = "maravilla.cloud"

[pgwire]
enabled = true
bind_address = "127.0.0.1"   # SSH tunnel for psql access
port = 5432
```

---

## Request Flow (How It Works End-to-End)

```
Browser: https://acme.maravilla.cloud/api/repository/myrepo/main/head/default/nodes
    |
    v
DNS: *.maravilla.cloud -> 65.x.x.x
    |
    v
Caddy: TLS termination, adds X-Forwarded-Host: acme.maravilla.cloud
    |
    v
RaisinDB (127.0.0.1:8080):
    |
    +-- ensure_tenant_middleware:
    |    Host = "acme.maravilla.cloud"
    |    base_domain = "maravilla.cloud"
    |    subdomain = "acme" -> tenant_id = "acme"
    |
    +-- unified_cors_middleware:
    |    Origin: https://acme.maravilla.cloud <- allowed (matches *.maravilla.cloud)
    |
    +-- optional_auth_middleware:
    |    Bearer token with tenant_id="acme" in claims
    |
    +-- Handler:
         NodeService::new(storage, "acme", "myrepo", "main", "default")
         -> RocksDB key: acme\0myrepo\0main\0default\0nodes\0...
```

---

## Provisioning a New Beta Customer

No self-service needed for beta. Just:

```bash
# 1. Create tenant (from your machine or SSH)
curl -X POST https://admin.maravilla.cloud/api/management/registry/tenants \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id": "acme", "metadata": {"plan": "beta", "owner": "john@acme.com"}}'

# 2. Configure auth for the tenant
curl -X PUT https://admin.maravilla.cloud/api/tenants/acme/auth/config \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "anonymous_enabled": false,
    "local_auth_enabled": true,
    "cors_allowed_origins": ["https://acme.maravilla.cloud", "http://localhost:5173"]
  }'

# 3. Customer visits https://acme.maravilla.cloud -- done!
#    (NodeTypes auto-initialized on first request)
```

---

## Growth Path

```
Beta (now)                    Later                         Scale
--------------------------------------------------------------------
Single instance               Add rate limiting             Second server
on one Hetzner AX42          + quota enforcement           + CRDT replication
                              + usage tracking              (already built)
                              + self-service portal
                              + Stripe billing

~2 GB RAM used                ~4-8 GB RAM used             2x servers
Unlimited beta tenants        50-100 paying tenants        1000+ tenants
```

---

## Deploying Updates (Downtime Story)

With a single process, a binary update means ~2-3 seconds of downtime for all tenants.

**Deploy script:**
```bash
#!/bin/bash
# deploy.sh - run on the Hetzner server
set -e

# 1. Copy new binary (built locally or in CI, cross-compiled for x86_64-linux)
cp /tmp/raisin-server-new /usr/local/bin/raisin-server

# 2. Restart (2-3 second gap, systemd restarts immediately)
systemctl restart raisindb

# 3. Health check
sleep 2
curl -f http://127.0.0.1:8080/health || echo "WARN: health check failed"
```

**Why this is fine for beta:**
- 2-3 seconds is invisible (browsers retry, WebSocket clients auto-reconnect)
- Deploy once a week at most, during off-hours if desired
- Compare to Firecracker: you'd need to rolling-restart 20 VMs individually (more complex, same per-tenant downtime)

**Zero-downtime option (if ever needed):**
- Blue-green deploy: start new binary on port 8081, switch Caddy upstream, kill old
- Or: implement graceful shutdown (axum's `with_graceful_shutdown`) so in-flight requests complete before exit

---

## Summary

- **Don't use Firecracker.** It's 10x more complex for zero benefit at beta scale.
- **Multi-tenant is already built.** You just need ~50 lines of subdomain parsing.
- **64 GB RAM is massive overkill.** A single RaisinDB instance will use 2-4 GB. You have room for years of growth.
- **Caddy + wildcard DNS** gives you instant HTTPS for any new subdomain.
- **Provisioning is 2 curl commands** per customer during beta.
- **The upgrade path** (rate limiting, quotas, dedicated instances) can be added incrementally.
