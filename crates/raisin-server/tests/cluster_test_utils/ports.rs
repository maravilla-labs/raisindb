// Port allocation utilities for cluster testing

use std::net::TcpListener;

/// Get a free port by binding to :0 and immediately releasing it
///
/// This is a best-effort approach - there's a small window where the port
/// could be taken between releasing and using it, but it's sufficient for tests.
pub fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .expect("Failed to bind to random port for port allocation");

    let port = listener
        .local_addr()
        .expect("Failed to get local address")
        .port();

    // Port is released when listener is dropped
    drop(listener);

    port
}

/// Allocate N unique free ports
///
/// Returns a vector of unique port numbers that were free at the time of allocation.
/// The ports are released immediately after allocation, so there's a small window
/// where they could be taken, but this is acceptable for test scenarios.
///
/// # Arguments
/// * `count` - Number of unique ports to allocate
///
/// # Returns
/// Vector of unique port numbers
pub fn unique_ports(count: usize) -> Vec<u16> {
    let mut ports = Vec::with_capacity(count);
    let mut seen = std::collections::HashSet::new();

    while ports.len() < count {
        let port = free_port();
        if seen.insert(port) {
            ports.push(port);
        }
    }

    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_free_port_allocates_port() {
        let port = free_port();
        assert!(port > 0);
        assert!(port < 65535);
    }

    #[test]
    fn test_unique_ports_allocates_different_ports() {
        let ports = unique_ports(6);
        assert_eq!(ports.len(), 6);

        // Verify all ports are unique
        let unique_count = ports.iter().collect::<std::collections::HashSet<_>>().len();
        assert_eq!(unique_count, 6);
    }

    #[test]
    fn test_unique_ports_all_valid() {
        let ports = unique_ports(10);
        for port in ports {
            assert!(port > 0);
            assert!(port < 65535);
        }
    }
}
