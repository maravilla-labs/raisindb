import com.raisindb.client.RaisinClient;
import com.raisindb.client.operations.*;
import com.raisindb.client.protocol.*;
import com.raisindb.client.events.Subscription;

import java.util.*;

/**
 * Basic usage example for RaisinDB Java client.
 */
public class BasicUsage {

    public static void main(String[] args) {
        try (RaisinClient client = new RaisinClient("raisin://localhost:8080/sys/default")) {

            // Connect to RaisinDB
            client.connect();
            System.out.println("✓ Connected to RaisinDB");

            // Authenticate
            client.authenticate("admin", "admin").get();
            System.out.println("✓ Authenticated");

            // Get database and workspace
            Database db = client.database("demo");
            Workspace workspace = db.workspace("content");

            // Create a node
            System.out.println("\nCreating a node...");
            Map<String, Object> properties = new HashMap<>();
            properties.put("title", "Welcome Page");
            properties.put("published", true);
            properties.put("author", "admin");

            Node node = workspace.nodes().create(
                    "Page",
                    "/welcome",
                    properties,
                    Map.of("body", "Welcome to RaisinDB!")
            ).get();

            System.out.println("✓ Created node: " + node.getNodeId());
            System.out.println("  Path: " + node.getPath());
            System.out.println("  Properties: " + node.getProperties());

            // Query nodes by path
            System.out.println("\nQuerying nodes...");
            List<Node> nodes = workspace.nodes().queryByPath("/welcome").get();
            System.out.println("✓ Found " + nodes.size() + " node(s)");
            for (Node n : nodes) {
                System.out.println("  - " + n.getPath() + ": " +
                        n.getProperties().get("title"));
            }

            // Update the node
            System.out.println("\nUpdating node...");
            Map<String, Object> updates = new HashMap<>();
            updates.put("title", "Updated Welcome Page");
            updates.put("published", true);

            Node updatedNode = workspace.nodes().update(
                    node.getNodeId(),
                    updates,
                    null
            ).get();
            System.out.println("✓ Updated node: " + updatedNode.getNodeId());
            System.out.println("  New title: " + updatedNode.getProperties().get("title"));

            // Execute SQL query
            System.out.println("\nExecuting SQL query...");
            SqlResult result = db.sql(
                    "SELECT * FROM nodes WHERE node_type = ?",
                    "Page"
            ).get();
            System.out.println("✓ Query returned " + result.getRowCount() + " row(s)");
            System.out.println("  Columns: " + result.getColumns());

            // List workspaces
            System.out.println("\nListing workspaces...");
            List<Object> workspaces = db.listWorkspaces().get();
            System.out.println("✓ Found " + workspaces.size() + " workspace(s)");

            // Subscribe to events
            System.out.println("\nSubscribing to events...");
            final int[] eventCount = {0};

            Subscription subscription = workspace.events().subscribe(
                    event -> {
                        eventCount[0]++;
                        System.out.println("\n📢 Event received: " + event.getEventType());
                        System.out.println("   Subscription: " + event.getSubscriptionId());
                        System.out.println("   Timestamp: " + event.getTimestamp());
                    },
                    "/",
                    List.of("node:created", "node:updated"),
                    null
            ).get();
            System.out.println("✓ Subscribed with ID: " + subscription.getSubscriptionId());

            // Create another node to trigger event
            System.out.println("\nCreating another node (should trigger event)...");
            Node node2 = workspace.nodes().create(
                    "Page",
                    "/test",
                    Map.of("title", "Test Page"),
                    null
            ).get();
            System.out.println("✓ Created node: " + node2.getNodeId());

            // Wait a bit for events
            Thread.sleep(1000);

            // Unsubscribe
            subscription.unsubscribe().get();
            System.out.println("✓ Unsubscribed (received " + eventCount[0] + " event(s))");

            // Clean up - delete test nodes
            System.out.println("\nCleaning up...");
            workspace.nodes().delete(node.getNodeId()).get();
            workspace.nodes().delete(node2.getNodeId()).get();
            System.out.println("✓ Deleted test nodes");

            System.out.println("\n✓ Example completed successfully!");

        } catch (Exception e) {
            System.err.println("✗ Error: " + e.getMessage());
            e.printStackTrace();
        }
    }
}
