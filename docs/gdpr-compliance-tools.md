# GDPR Compliance Built-in Tools for RaisinDB

## Executive Summary

RaisinDB, as a general-purpose AI database repository handling end-user (identity) data, must provide built-in GDPR compliance tools to its clients. This document outlines the recommended built-in features based on:
- European GDPR requirements (Articles 12-22, 30, 32-34)
- Current RaisinDB architecture analysis
- Industry best practices for database compliance

---

## Current Architecture Strengths

RaisinDB already has strong foundations:
- **Multi-tenant isolation** - All data scoped by tenant_id
- **Identity model** - Comprehensive with `created_by`, `owner_id` tracking
- **Audit infrastructure** - AuditLog model, OpLog, NodeVersions
- **Permission system** - Row-level security (RLS), field-level filtering
- **Soft delete** - Tombstone-based deletion preserving history

---

## Recommended Built-in GDPR Tools

### 1. Data Subject Access Request (DSAR) Tool (Article 15)

**Purpose**: Allow end-users to request all data held about them.

**Built-in Features**:
```
GET /gdpr/identity/{identity_id}/export
```

| Feature | Description |
|---------|-------------|
| **Identity Export** | All identity data, linked providers, metadata |
| **Content Export** | All nodes where `created_by` or `owner_id` = identity |
| **Activity Export** | Audit logs, session history, login history |
| **Format Options** | JSON (machine-readable), PDF (human-readable) |
| **Cascading Discovery** | Traverse: Identity → WorkspaceAccess → Nodes → Versions |

**Implementation**: Build on existing `NodeRepository`, `IdentityRepository`, traverse relationships via `created_by`/`owner_id` fields.

---

### 2. Right to Erasure Tool (Article 17 - "Right to be Forgotten")

**Purpose**: Permanently delete all personal data upon request.

**Built-in Features**:
```
DELETE /gdpr/identity/{identity_id}/erase
POST /gdpr/identity/{identity_id}/anonymize
```

| Feature | Description |
|---------|-------------|
| **Hard Delete Mode** | Physically remove data (not just tombstone) |
| **Anonymization Mode** | Replace PII with anonymized values, preserve structure |
| **Cascade Deletion** | Delete: Identity → Sessions → Tokens → AccessRecords → Content |
| **Deletion Certificate** | Cryptographic proof of deletion for compliance records |
| **Retention Override** | Skip legal-hold data, document exceptions |

**Deletion Cascade Order**:
1. Active sessions (revoke all)
2. One-time tokens
3. WorkspaceAccess records
4. Nodes (where `created_by` = identity) - anonymize or delete
5. NodeVersions (historical snapshots)
6. AuditLogs (anonymize actor, keep for compliance)
7. Identity record itself

**Gap to Address**: Current soft-delete needs hard-delete option + garbage collection implementation.

---

### 3. Data Portability Tool (Article 20)

**Purpose**: Export data in machine-readable format for transfer to another service.

**Built-in Features**:
```
GET /gdpr/identity/{identity_id}/portable-export
```

| Feature | Description |
|---------|-------------|
| **Standard Formats** | JSON-LD, CSV, XML |
| **Schema Export** | Include NodeType definitions for context |
| **Binary Assets** | Include or reference (with signed URLs) |
| **Incremental Export** | Export changes since last export |
| **Import Endpoint** | Accept portable format from other systems |

---

### 4. Consent Management Tool (Articles 6, 7)

**Purpose**: Track and manage user consent for data processing.

**New Model Required**:
```rust
pub struct ConsentRecord {
    pub id: String,
    pub identity_id: String,
    pub tenant_id: String,
    pub purpose: ConsentPurpose,        // Marketing, Analytics, etc.
    pub status: ConsentStatus,          // Granted, Withdrawn, Expired
    pub granted_at: Option<Timestamp>,
    pub withdrawn_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub version: String,                // Policy version consented to
    pub source: ConsentSource,          // Web, API, Import
    pub ip_address: Option<String>,
    pub proof: Option<String>,          // Signature or audit reference
}
```

**Built-in Features**:
```
POST /gdpr/consent/{identity_id}/grant
POST /gdpr/consent/{identity_id}/withdraw
GET  /gdpr/consent/{identity_id}/history
```

| Feature | Description |
|---------|-------------|
| **Purpose-Based Consent** | Marketing, Analytics, Profiling, Third-Party Sharing |
| **Consent Versioning** | Track which policy version was consented to |
| **Withdrawal Tracking** | Record when/how consent was withdrawn |
| **Audit Trail** | Immutable consent history |
| **Expiration** | Auto-expire consent after configurable period |

---

### 5. Data Retention Policy Engine (Article 5(1)(e))

**Purpose**: Automatically enforce data retention limits.

**Built-in Features**:
```yaml
# Example retention policy configuration
retention_policies:
  - name: "session_data"
    target: "sessions"
    max_age: "90d"
    action: "delete"

  - name: "audit_logs"
    target: "audit_logs"
    max_age: "7y"           # Legal requirement
    action: "archive"

  - name: "inactive_users"
    target: "identities"
    condition: "last_login_at < now() - 2y"
    action: "anonymize"
    notify_before: "30d"
```

| Feature | Description |
|---------|-------------|
| **Policy Configuration** | Define retention rules per data type |
| **Automatic Enforcement** | Job-based cleanup via JobRegistry |
| **Legal Hold Override** | Suspend retention for litigation |
| **Pre-Deletion Notification** | Warn users before auto-deletion |
| **Retention Reports** | Compliance reporting on retention adherence |

**Implementation**: Leverage existing `delete_old_versions()` pattern, extend to all data types.

---

### 6. Access Logging & Audit Trail (Article 30)

**Purpose**: Record all access to personal data.

**Enhance Existing AuditLog**:
```rust
pub struct GdprAuditEntry {
    pub id: String,
    pub timestamp: Timestamp,
    pub actor_id: String,           // Who accessed
    pub actor_type: ActorType,      // User, System, Admin
    pub action: GdprAction,         // Read, Export, Modify, Delete
    pub data_subject_id: String,    // Whose data was accessed
    pub data_categories: Vec<String>, // What types of data
    pub legal_basis: String,        // Why access was allowed
    pub tenant_id: String,
    pub ip_address: Option<String>,
    pub retention_until: Timestamp,
}
```

**Built-in Features**:
```
GET /gdpr/audit/{identity_id}/access-log
GET /gdpr/audit/report?from=&to=
```

| Feature | Description |
|---------|-------------|
| **Persistent Storage** | Store in RocksDB column family (not in-memory) |
| **Immutable Entries** | Append-only, no modification |
| **Query by Subject** | "Who accessed this user's data?" |
| **Query by Actor** | "What data did this admin access?" |
| **Compliance Reports** | Generate Article 30 records |

**Gap**: Current AuditRepository is in-memory. Need persistent implementation.

---

### 7. Data Anonymization Service (Article 89)

**Purpose**: Remove identifying information while preserving data utility.

**Built-in Features**:
```
POST /gdpr/anonymize/node/{node_id}
POST /gdpr/anonymize/identity/{identity_id}
```

| Technique | Use Case |
|-----------|----------|
| **Pseudonymization** | Replace identity with reversible token |
| **Generalization** | "John Smith, 34" → "Male, 30-40" |
| **Suppression** | Remove field entirely |
| **Data Masking** | "john@example.com" → "j***@e***.com" |
| **K-Anonymity** | Ensure data can't identify individuals |

**Implementation**: Field-level transformers applied during export or in-place.

---

### 8. Breach Notification Support (Articles 33, 34)

**Purpose**: Facilitate 72-hour breach notification requirement.

**Built-in Features**:
```
POST /gdpr/breach/report
GET  /gdpr/breach/{breach_id}/affected-subjects
POST /gdpr/breach/{breach_id}/notify
```

| Feature | Description |
|---------|-------------|
| **Breach Registration** | Record breach details, scope, timeline |
| **Impact Assessment** | Query affected identities by data category |
| **Notification Templates** | Pre-built templates for DPA and subjects |
| **72-Hour Timer** | Track notification deadlines |
| **Evidence Export** | Package affected data for investigation |

---

### 9. Privacy by Design Configuration (Article 25)

**Purpose**: Default privacy-protective settings.

**Built-in Features**:

| Setting | Default | Description |
|---------|---------|-------------|
| `gdpr.data_minimization` | `true` | Warn when collecting non-essential fields |
| `gdpr.purpose_limitation` | `true` | Require purpose for each data collection |
| `gdpr.storage_limitation` | `true` | Enforce retention policies |
| `gdpr.default_encryption` | `true` | Encrypt PII fields at rest |
| `gdpr.pseudonymization` | `false` | Auto-pseudonymize external references |
| `gdpr.audit_all_access` | `true` | Log all personal data access |

---

### 10. Processing Records Register (Article 30)

**Purpose**: Maintain records of processing activities.

**Built-in Features**:
```
GET /gdpr/processing-register
```

**Auto-generated from configuration**:
```yaml
processing_activities:
  - name: "User Authentication"
    purposes: ["Security", "Access Control"]
    data_categories: ["Email", "Password Hash", "Login History"]
    legal_basis: "Contract Performance"
    retention: "Account Lifetime + 90 days"
    recipients: ["Internal Only"]

  - name: "Content Creation"
    purposes: ["Service Delivery"]
    data_categories: ["User Content", "Metadata"]
    legal_basis: "Contract Performance"
    retention: "User Configurable"
    recipients: ["Internal", "Configured Integrations"]
```

---

## Implementation Priority

### Phase 1: Critical (Must-Have)
1. **DSAR Export Tool** - Most common request
2. **Right to Erasure** - Legal requirement
3. **Persistent Audit Logging** - Foundation for all compliance

### Phase 2: Important (Should-Have)
4. **Consent Management** - Required for lawful processing
5. **Data Retention Engine** - Automated compliance
6. **Anonymization Service** - Supports erasure alternatives

### Phase 3: Enhancement (Nice-to-Have)
7. **Data Portability** - Less commonly requested
8. **Breach Notification** - Hopefully rarely needed
9. **Processing Register** - Documentation tool
10. **Privacy Config** - Operational guidance

---

## Technical Implementation Notes

### Leverage Existing Infrastructure

| Existing | Use For |
|----------|---------|
| `JobRegistry` + `JobDataStore` | Async GDPR operations (export, erasure) |
| `OpLog` | Audit trail foundation |
| `NodeVersions` | Historical data for portability |
| `RLS Filter` | Field-level anonymization |
| `IdentityRepository` | Subject discovery |
| `created_by`/`owner_id` | Data ownership traversal |

### New Components Needed

1. **GdprService** - Orchestrates all GDPR operations
2. **ConsentRepository** - Stores consent records
3. **GdprAuditRepository** - Persistent audit storage
4. **AnonymizationEngine** - Field transformers
5. **RetentionPolicyRunner** - Scheduled job for retention enforcement
6. **BreachManager** - Breach tracking and notification

### API Surface

```
/gdpr/
  /identity/{id}/export          # DSAR
  /identity/{id}/erase           # Erasure
  /identity/{id}/anonymize       # Anonymization
  /identity/{id}/portable-export # Portability

  /consent/{id}/grant            # Consent management
  /consent/{id}/withdraw
  /consent/{id}/history

  /audit/{id}/access-log         # Audit trail
  /audit/report

  /retention/policies            # Retention management
  /retention/run

  /breach/report                 # Breach handling
  /breach/{id}/affected-subjects
  /breach/{id}/notify

  /processing-register           # Documentation
```

---

## Summary

RaisinDB should provide these **10 built-in GDPR tools**:

1. **DSAR Export** - Data subject access requests
2. **Right to Erasure** - Deletion and anonymization
3. **Data Portability** - Machine-readable export/import
4. **Consent Management** - Track and manage consent
5. **Retention Policy Engine** - Automated data lifecycle
6. **Access Audit Trail** - Who accessed what, when
7. **Anonymization Service** - PII transformation
8. **Breach Notification** - 72-hour compliance support
9. **Privacy by Design Config** - Default-safe settings
10. **Processing Register** - Article 30 documentation

These tools transform GDPR compliance from a client burden into a platform capability, making RaisinDB a trusted choice for European data processing.
