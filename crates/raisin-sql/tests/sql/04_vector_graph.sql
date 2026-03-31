-- Vector search and graph traversal queries

-- KNN: K-nearest neighbors vector search (table-valued function)
-- Note: KNN is used in FROM clause as a table-valued function

-- Basic KNN query with JOIN
SELECT n.id, n.name, knn.distance
FROM KNN(:query_vec, 10) AS knn
JOIN nodes n ON n.id = knn.node_id;

-- KNN with filtering
SELECT n.id, n.name, n.properties ->> 'title' AS title, knn.distance
FROM KNN(:query_vec, 20, 'node_type = ''my:Article''') AS knn
JOIN nodes n ON n.id = knn.node_id
ORDER BY knn.distance;

-- KNN with additional WHERE conditions
SELECT n.id, n.name, knn.distance
FROM KNN(:query_vec, 10) AS knn
JOIN nodes n ON n.id = knn.node_id
WHERE n.properties ->> 'status' = 'published';

-- NEIGHBORS: Graph traversal (table-valued function)

-- Basic NEIGHBORS query - outgoing edges
SELECT n.name
FROM NEIGHBORS(:event_id, 'OUT', 'ORGANIZED_BY') AS e
JOIN nodes n ON n.id = e.dst_id;

-- NEIGHBORS - incoming edges
SELECT n.name
FROM NEIGHBORS(:person_id, 'IN', 'AUTHORED_BY') AS e
JOIN nodes n ON n.id = e.src_id;

-- NEIGHBORS - both directions
SELECT n.id, n.name, e.edge_label
FROM NEIGHBORS(:node_id, 'BOTH', NULL) AS e
JOIN nodes n ON n.id = CASE
    WHEN e.direction = 'OUT' THEN e.dst_id
    WHEN e.direction = 'IN' THEN e.src_id
END;

-- NEIGHBORS with edge properties
SELECT
    n.name,
    e.edge_label,
    e.edge_properties ->> 'since' AS relationship_since
FROM NEIGHBORS(:user_id, 'OUT', 'FOLLOWS') AS e
JOIN nodes n ON n.id = e.dst_id;

-- Multi-hop graph traversal (2 levels)
SELECT n2.id, n2.name
FROM NEIGHBORS(:start_node, 'OUT', 'KNOWS') AS e1
JOIN nodes n1 ON n1.id = e1.dst_id
CROSS JOIN LATERAL NEIGHBORS(n1.id, 'OUT', 'KNOWS') AS e2
JOIN nodes n2 ON n2.id = e2.dst_id;

-- Combining KNN with graph traversal
SELECT
    n.id,
    n.name,
    knn.distance,
    COUNT(e.dst_id) as connection_count
FROM KNN(:query_vec, 50) AS knn
JOIN nodes n ON n.id = knn.node_id
LEFT JOIN NEIGHBORS(n.id, 'OUT', 'RELATED_TO') AS e ON true
GROUP BY n.id, n.name, knn.distance
ORDER BY knn.distance;

-- Graph traversal with hierarchy
SELECT
    n.id,
    n.name,
    n.path,
    e.edge_label
FROM NEIGHBORS(:node_id, 'OUT', NULL) AS e
JOIN nodes n ON n.id = e.dst_id
WHERE PATH_STARTS_WITH(n.path, '/content/');

-- Complex vector search with JSON filtering
SELECT
    n.id,
    n.name,
    properties ->> 'title' AS title,
    knn.distance
FROM KNN(:embedding, 100) AS knn
JOIN nodes n ON n.id = knn.node_id
WHERE JSON_EXISTS(properties, '$.tags')
AND properties @> '{"status": "published"}'
ORDER BY knn.distance
LIMIT 10;
