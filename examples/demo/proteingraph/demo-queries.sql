-- ============================================================================
-- ProteinGraph Demo Queries: The 3 Wow Moments
-- ============================================================================
--
-- Run these queries in psql, DBeaver, or Jupyter to showcase RaisinDB's
-- graph capabilities for bioinformatics.
--
-- Connection String Format:
--   postgresql://{tenant}:{api_key}@{host}:{port}/{repository}
--
-- Example:
--   postgresql://default:raisin_XXXXXXXXXXXX@localhost:5432/proteingraph
--
-- Usage:
--   export RAISIN_API_KEY="raisin_YOUR_API_KEY_HERE"
--   psql "postgresql://default:${RAISIN_API_KEY}@localhost:5432/proteingraph" -f demo-queries.sql
--
-- ============================================================================


-- ============================================================================
-- WOW #1: "SQL that speaks Graph" (2-3 min)
-- ============================================================================
-- Show a query that would be impossible/ugly in traditional SQL.
-- Hook: "No Neo4j. No Cypher. Just SQL. ISO SQL:2023 standard."
-- ============================================================================

-- Query 1a: Find all proteins that interact with APP (single hop)
-- "What proteins directly interact with APP?"
SELECT * FROM GRAPH_TABLE(
    MATCH (app:Protein)-[r:INTERACTS_WITH]->(partner:Protein)
    WHERE app.path = '/alzheimer-study/proteins/APP'
    COLUMNS (
        partner.name AS interacting_protein,
        partner.properties->>'gene_id' AS gene_symbol,
        r.weight AS confidence_score
    )
) AS interactions
ORDER BY confidence_score DESC;


-- Query 1b: Variable-length paths - The killer feature!
-- "Find proteins 2-3 hops away from APP through the interaction network"
SELECT * FROM GRAPH_TABLE(
    MATCH (start:Protein)-[:INTERACTS_WITH*2..3]->(distant:Protein)
    WHERE start.path = '/alzheimer-study/proteins/APP'
    COLUMNS (
        start.properties->>'gene_id' AS source,
        distant.properties->>'gene_id' AS target,
        distant.name AS target_name
    )
) AS distant_proteins
LIMIT 20;


-- Query 1c: Drug target discovery
-- "Which proteins are targeted by Aducanumab and what do they interact with?"
SELECT * FROM GRAPH_TABLE(
    MATCH (drug:Drug)-[:TARGETS]->(target:Protein)-[:INTERACTS_WITH*1..2]->(downstream:Protein)
    WHERE drug.path = '/alzheimer-study/drugs/ADUCANUMAB'
    COLUMNS (
        drug.name AS drug_name,
        target.properties->>'gene_id' AS direct_target,
        downstream.properties->>'gene_id' AS downstream_protein,
        downstream.properties->>'druggable' AS is_druggable
    )
) AS drug_network
ORDER BY is_druggable DESC;


-- Query 1d: Find proteins associated with Alzheimer's and their interactions
-- "Show me proteins linked to Alzheimer's that interact with APP"
SELECT * FROM GRAPH_TABLE(
    MATCH (p:Protein)-[:ASSOCIATED_WITH]->(d:Disease),
          (p)-[:INTERACTS_WITH]-(app:Protein)
    WHERE d.path = '/alzheimer-study/diseases/ALZHEIMER'
      AND app.path = '/alzheimer-study/proteins/APP'
    COLUMNS (
        p.properties->>'gene_id' AS gene,
        p.name AS protein_name,
        p.properties->>'druggable' AS druggable
    )
) AS disease_related
ORDER BY druggable DESC;


-- ============================================================================
-- WOW #2: "Find Similar Proteins" (2 min)
-- ============================================================================
-- Vector similarity with protein embeddings.
-- Hook: "Protein language models + graph DB in one query"
--
-- NOTE: This requires embeddings to be loaded. For demo purposes,
-- we show the query structure. In production, you'd load ESM-2 or
-- text-embedding-3-small vectors.
-- ============================================================================

-- Query 2a: Find proteins with similar descriptions (text similarity)
-- Using fulltext search as a proxy for similarity
SELECT
    path,
    name,
    properties->>'gene_id' AS gene,
    properties->>'description' AS description
FROM default
WHERE node_type = 'bio:Protein'
  AND FULLTEXT_MATCH(properties->>'description', 'amyloid secretase')
ORDER BY name;


-- Query 2b: Vector similarity query structure (when embeddings are loaded)
-- This is the query you'd run with real embeddings:
--
-- SELECT
--     target.path,
--     target.properties->>'gene_id' AS gene,
--     VECTOR_L2_DISTANCE(
--         source.properties->'embedding',
--         target.properties->'embedding'
--     ) AS distance
-- FROM default source, default target
-- WHERE source.path = '/alzheimer-study/proteins/APP'
--   AND target.node_type = 'bio:Protein'
--   AND target.path != source.path
--   AND VECTOR_L2_DISTANCE(
--         source.properties->'embedding',
--         target.properties->'embedding'
--       ) < 0.5
-- ORDER BY distance
-- LIMIT 10;


-- Query 2c: Find druggable proteins in the gamma-secretase complex
-- Combining graph traversal with property filtering
SELECT * FROM GRAPH_TABLE(
    MATCH (psen:Protein)-[:INTERACTS_WITH]-(partner:Protein)
    WHERE psen.path = '/alzheimer-study/proteins/PSEN1'
    COLUMNS (
        partner.properties->>'gene_id' AS gene,
        partner.name AS protein,
        partner.properties->>'druggable' AS druggable,
        partner.properties->>'description' AS description
    )
) AS complex
WHERE druggable = 'true';


-- ============================================================================
-- WOW #3: "Use Your Existing Tools" (1-2 min)
-- ============================================================================
-- The same queries work in psql, DBeaver, Jupyter, R, Python...
-- Hook: "Works with everything you already use. No new clients."
--
-- These queries demonstrate RaisinDB's PostgreSQL compatibility.
-- ============================================================================

-- Query 3a: Simple count query - works exactly like PostgreSQL
SELECT
    node_type,
    COUNT(*) as count
FROM default
WHERE path LIKE '/alzheimer-study/%'
GROUP BY node_type
ORDER BY count DESC;


-- Query 3b: JSON property access - PostgreSQL JSONB syntax
SELECT
    name,
    properties->>'gene_id' AS gene,
    properties->>'chromosome' AS chr,
    (properties->>'molecular_weight')::INTEGER AS mw,
    properties->>'druggable' AS druggable
FROM default
WHERE node_type = 'bio:Protein'
  AND (properties->>'druggable')::BOOLEAN = true
ORDER BY mw DESC;


-- Query 3c: Find all approved Alzheimer's drugs and their targets
SELECT
    d.name AS drug,
    d.properties->>'brand_name' AS brand,
    d.properties->>'approval_year' AS year,
    STRING_AGG(t.properties->>'gene_id', ', ') AS targets
FROM default d
JOIN NEIGHBORS('default:' || d.path, 'OUT', 'TARGETS') AS n ON true
JOIN default t ON t.path = n.path
WHERE d.node_type = 'bio:Drug'
  AND (d.properties->>'fda_approved')::BOOLEAN = true
GROUP BY d.name, d.properties->>'brand_name', d.properties->>'approval_year'
ORDER BY (d.properties->>'approval_year')::INTEGER DESC;


-- Query 3d: Hierarchical query - get all content under a path
SELECT
    path,
    node_type,
    name
FROM default
WHERE DESCENDANT_OF('/alzheimer-study')
ORDER BY path;


-- ============================================================================
-- BONUS QUERIES: Advanced Graph Analysis
-- ============================================================================

-- Bonus 1: Find hub proteins (most connections)
SELECT * FROM GRAPH_TABLE(
    MATCH (p:Protein)-[r:INTERACTS_WITH]-(other:Protein)
    COLUMNS (
        p.properties->>'gene_id' AS gene,
        p.name AS protein,
        COUNT(*) AS connection_count
    )
) AS hubs
ORDER BY connection_count DESC
LIMIT 10;


-- Bonus 2: Find shortest path between two proteins
-- "How is BACE1 connected to MAPT (tau)?"
SELECT * FROM GRAPH_TABLE(
    MATCH (start:Protein)-[:INTERACTS_WITH*1..4]->(end:Protein)
    WHERE start.path = '/alzheimer-study/proteins/BACE1'
      AND end.path = '/alzheimer-study/proteins/MAPT'
    COLUMNS (
        start.properties->>'gene_id' AS from_protein,
        end.properties->>'gene_id' AS to_protein
    )
) AS paths
LIMIT 5;


-- Bonus 3: Find proteins that are both drug targets and disease-associated
SELECT * FROM GRAPH_TABLE(
    MATCH (drug:Drug)-[:TARGETS]->(p:Protein)-[:ASSOCIATED_WITH]->(disease:Disease)
    WHERE disease.path = '/alzheimer-study/diseases/ALZHEIMER'
    COLUMNS (
        drug.name AS drug,
        p.properties->>'gene_id' AS target_gene,
        p.name AS protein
    )
) AS drug_disease_link;


-- ============================================================================
-- End of Demo Queries
-- ============================================================================
--
-- Key talking points:
-- 1. ISO SQL:2023 Part 16 - Graph queries are now SQL standard
-- 2. Variable-length paths with *1..3 syntax
-- 3. Works with standard PostgreSQL tools (psql, Jupyter, DBeaver)
-- 4. Combine graph patterns with SQL aggregations and JOINs
-- 5. JSONB property access for flexible schema
--
-- ============================================================================
