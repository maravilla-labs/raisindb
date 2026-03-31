// SPDX-License-Identifier: BSL-1.1

//! Demonstration of HLC usage in a multi-node replication scenario
//!
//! This example simulates a 3-node distributed system where nodes generate
//! operations and replicate them to each other, showing how HLC maintains
//! causal consistency.
//!
//! Run with: cargo run --package raisin-hlc --example replication_demo

use raisin_hlc::{NodeHLCState, HLC};

/// Represents an operation in the distributed system
#[derive(Debug, Clone)]
struct Operation {
    node_id: String,
    hlc: HLC,
    data: String,
}

/// Simulates a node in the distributed system
struct Node {
    id: String,
    hlc_state: NodeHLCState,
    operations: Vec<Operation>,
}

impl Node {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            hlc_state: NodeHLCState::new(id.to_string()),
            operations: Vec::new(),
        }
    }

    /// Generate a new local operation
    fn create_operation(&mut self, data: String) -> Operation {
        let hlc = self.hlc_state.tick();
        let op = Operation {
            node_id: self.id.clone(),
            hlc,
            data,
        };
        self.operations.push(op.clone());
        println!(
            "[{}] Created operation: {} (HLC: {})",
            self.id, op.data, op.hlc
        );
        op
    }

    /// Receive and apply a remote operation
    fn receive_operation(&mut self, op: Operation) {
        println!(
            "[{}] Receiving operation from {}: {} (HLC: {})",
            self.id, op.node_id, op.data, op.hlc
        );

        // Update local clock to maintain causality
        let updated_hlc = self.hlc_state.update(&op.hlc);
        println!("[{}] Local clock updated to: {}", self.id, updated_hlc);

        self.operations.push(op);
    }

    /// Get all operations sorted by HLC (causal order)
    fn get_sorted_operations(&self) -> Vec<Operation> {
        let mut ops = self.operations.clone();
        ops.sort_by_key(|op| op.hlc);
        ops
    }

    /// Display the current state
    fn display_state(&self) {
        println!("\n[{}] Current state:", self.id);
        println!("  Current HLC: {}", self.hlc_state.current());
        println!("  Operations (causal order):");
        for op in self.get_sorted_operations() {
            println!("    {} | {} | {}", op.hlc, op.node_id, op.data);
        }
    }
}

fn main() {
    println!("=== HLC Replication Demo ===\n");

    // Initialize 3 nodes
    let mut node1 = Node::new("Node-1");
    let mut node2 = Node::new("Node-2");
    let mut node3 = Node::new("Node-3");

    println!("Step 1: Node-1 creates an operation\n");
    let op1 = node1.create_operation("User created post A".to_string());

    println!("\nStep 2: Node-1's operation replicates to Node-2\n");
    node2.receive_operation(op1.clone());

    println!("\nStep 3: Node-2 creates a dependent operation\n");
    let op2 = node2.create_operation("User liked post A".to_string());
    println!(
        "  Note: op2.hlc ({}) > op1.hlc ({}) ensures causality",
        op2.hlc, op1.hlc
    );

    println!("\nStep 4: Both operations replicate to Node-3\n");
    node3.receive_operation(op1.clone());
    node3.receive_operation(op2.clone());

    println!("\nStep 5: Node-3 creates its own operation\n");
    let op3 = node3.create_operation("User commented on post A".to_string());

    println!("\nStep 6: Node-3's operation replicates to Node-1 and Node-2\n");
    node1.receive_operation(op2.clone());
    node1.receive_operation(op3.clone());
    node2.receive_operation(op3.clone());

    // Display final state of all nodes
    println!("\n{}", "=".repeat(60));
    println!("Final State of All Nodes");
    println!("{}", "=".repeat(60));

    node1.display_state();
    node2.display_state();
    node3.display_state();

    // Verify causal consistency
    println!("\n{}", "=".repeat(60));
    println!("Causal Consistency Verification");
    println!("{}\n", "=".repeat(60));

    let ops1 = node1.get_sorted_operations();
    let ops2 = node2.get_sorted_operations();
    let ops3 = node3.get_sorted_operations();

    // All nodes should have the same causal order
    let hlcs1: Vec<HLC> = ops1.iter().map(|op| op.hlc).collect();
    let hlcs2: Vec<HLC> = ops2.iter().map(|op| op.hlc).collect();
    let hlcs3: Vec<HLC> = ops3.iter().map(|op| op.hlc).collect();

    assert_eq!(
        hlcs1, hlcs2,
        "Node-1 and Node-2 should have same causal order"
    );
    assert_eq!(
        hlcs2, hlcs3,
        "Node-2 and Node-3 should have same causal order"
    );

    println!("✓ All nodes have consistent causal ordering!");
    println!("  Order: {} < {} < {}", op1.hlc, op2.hlc, op3.hlc);

    // Demonstrate encoding
    println!("\n{}", "=".repeat(60));
    println!("Encoding Demonstration");
    println!("{}\n", "=".repeat(60));

    for (i, op) in [op1, op2, op3].iter().enumerate() {
        let encoded = op.hlc.encode_descending();
        println!("Operation {}: HLC = {}", i + 1, op.hlc);
        println!("  Binary (descending): {:02x?}", &encoded[..8]);
        println!("  String format: {}", op.hlc);
        println!("  U128 format: {}", op.hlc.as_u128());
        println!();
    }

    // Demonstrate that newer HLCs sort first in descending encoding
    println!("Descending encoding verification:");
    let enc1 = ops1[0].hlc.encode_descending();
    let enc2 = ops1[1].hlc.encode_descending();
    let enc3 = ops1[2].hlc.encode_descending();

    println!(
        "  op1.hlc = {}, encoded[0..4] = {:02x?}",
        ops1[0].hlc,
        &enc1[0..4]
    );
    println!(
        "  op2.hlc = {}, encoded[0..4] = {:02x?}",
        ops1[1].hlc,
        &enc2[0..4]
    );
    println!(
        "  op3.hlc = {}, encoded[0..4] = {:02x?}",
        ops1[2].hlc,
        &enc3[0..4]
    );

    assert!(enc2 < enc1, "Newer HLC should have smaller bytes");
    assert!(enc3 < enc2, "Newer HLC should have smaller bytes");
    println!("\n✓ Descending encoding verified: newest operations sort first!");

    // Performance demonstration
    println!("\n{}", "=".repeat(60));
    println!("Performance Demonstration");
    println!("{}\n", "=".repeat(60));

    let state = NodeHLCState::new("perf-test".to_string());
    let iterations = 100_000;

    let start = std::time::Instant::now();
    for _ in 0..iterations {
        state.tick();
    }
    let duration = start.elapsed();

    let avg_ns = duration.as_nanos() / iterations as u128;
    println!("Generated {} timestamps in {:?}", iterations, duration);
    println!("Average time per tick: {}ns", avg_ns);
    println!(
        "Throughput: {:.2} million ops/sec",
        iterations as f64 / duration.as_secs_f64() / 1_000_000.0
    );
}
