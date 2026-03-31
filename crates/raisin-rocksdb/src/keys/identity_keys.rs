//! Identity and session key functions
//!
//! Keys for the pluggable authentication system: identities, sessions,
//! email indexes, and identity-session relationships.

use super::KeyBuilder;

/// Identity key: {tenant}\0identities\0{identity_id}
pub fn identity_key(tenant_id: &str, identity_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("identities")
        .push(identity_id)
        .build()
}

/// Identity prefix: {tenant}\0identities\0
pub fn identity_prefix(tenant_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("identities")
        .build_prefix()
}

/// Identity email index key: {tenant}\0identity_email\0{email}
pub fn identity_email_index_key(tenant_id: &str, email: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("identity_email")
        .push(email)
        .build()
}

/// Session key: {tenant}\0sessions\0{session_id}
pub fn session_key(tenant_id: &str, session_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("sessions")
        .push(session_id)
        .build()
}

/// Session prefix: {tenant}\0sessions\0
pub fn session_prefix(tenant_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("sessions")
        .build_prefix()
}

/// Identity sessions index key: {tenant}\0identity_sessions\0{identity_id}\0{session_id}
pub fn identity_sessions_index_key(
    tenant_id: &str,
    identity_id: &str,
    session_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("identity_sessions")
        .push(identity_id)
        .push(session_id)
        .build()
}

/// Identity sessions index prefix: {tenant}\0identity_sessions\0{identity_id}\0
pub fn identity_sessions_prefix(tenant_id: &str, identity_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push("identity_sessions")
        .push(identity_id)
        .build_prefix()
}
