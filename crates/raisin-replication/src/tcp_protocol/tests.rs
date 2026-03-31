//! Tests for TCP protocol message types

use super::*;
use uuid::Uuid;

#[test]
fn test_hello_message_roundtrip() {
    let msg = ReplicationMessage::hello("node1".to_string());
    let bytes = msg.to_bytes().unwrap();
    let decoded = ReplicationMessage::from_bytes(&bytes).unwrap();

    match decoded {
        ReplicationMessage::Hello {
            cluster_node_id,
            protocol_version,
            ..
        } => {
            assert_eq!(cluster_node_id, "node1");
            assert_eq!(protocol_version, PROTOCOL_VERSION);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_ping_pong_roundtrip() {
    let ping = ReplicationMessage::ping();
    let timestamp = match &ping {
        ReplicationMessage::Ping { timestamp_ms } => *timestamp_ms,
        _ => panic!("Wrong message type"),
    };

    let pong = ReplicationMessage::pong(timestamp);
    let bytes = pong.to_bytes().unwrap();
    let decoded = ReplicationMessage::from_bytes(&bytes).unwrap();

    match decoded {
        ReplicationMessage::Pong { timestamp_ms } => {
            assert_eq!(timestamp_ms, timestamp);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_error_message() {
    let msg = ReplicationMessage::error(
        ErrorCode::ProtocolVersionMismatch,
        "Protocol version mismatch".to_string(),
    );
    let bytes = msg.to_bytes().unwrap();
    let decoded = ReplicationMessage::from_bytes(&bytes).unwrap();

    match decoded {
        ReplicationMessage::Error { code, message, .. } => {
            assert_eq!(code, ErrorCode::ProtocolVersionMismatch);
            assert_eq!(message, "Protocol version mismatch");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_ack_message() {
    let op_ids = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    let msg = ReplicationMessage::ack(op_ids.clone());
    let bytes = msg.to_bytes().unwrap();
    let decoded = ReplicationMessage::from_bytes(&bytes).unwrap();

    match decoded {
        ReplicationMessage::Ack {
            op_ids: decoded_ids,
        } => {
            assert_eq!(decoded_ids, op_ids);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_encode_with_length() {
    let msg = ReplicationMessage::hello("node1".to_string());
    let encoded = msg.encode_with_length().unwrap();

    // First 4 bytes should be length
    let len = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]) as usize;
    assert_eq!(len, encoded.len() - 4);
}

#[test]
fn test_get_vector_clock_message() {
    let msg = ReplicationMessage::GetVectorClock {
        tenant_id: "tenant1".to_string(),
        repo_id: "repo1".to_string(),
    };

    let bytes = msg.to_bytes().unwrap();
    let decoded = ReplicationMessage::from_bytes(&bytes).unwrap();

    match decoded {
        ReplicationMessage::GetVectorClock { tenant_id, repo_id } => {
            assert_eq!(tenant_id, "tenant1");
            assert_eq!(repo_id, "repo1");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_pull_operations_message() {
    use crate::VectorClock;

    let mut vc = VectorClock::new();
    vc.increment("node1");
    vc.increment("node2");

    let msg = ReplicationMessage::PullOperations {
        tenant_id: "tenant1".to_string(),
        repo_id: "repo1".to_string(),
        since_vector_clock: vc.clone(),
        branch_filter: Some(vec!["main".to_string(), "develop".to_string()]),
        limit: 500,
    };

    let bytes = msg.to_bytes().unwrap();
    let decoded = ReplicationMessage::from_bytes(&bytes).unwrap();

    match decoded {
        ReplicationMessage::PullOperations {
            tenant_id,
            repo_id,
            since_vector_clock,
            branch_filter,
            limit,
        } => {
            assert_eq!(tenant_id, "tenant1");
            assert_eq!(repo_id, "repo1");
            assert_eq!(since_vector_clock, vc);
            assert_eq!(
                branch_filter,
                Some(vec!["main".to_string(), "develop".to_string()])
            );
            assert_eq!(limit, 500);
        }
        _ => panic!("Wrong message type"),
    }
}
