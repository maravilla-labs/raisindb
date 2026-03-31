-- ============================================================================
-- ProteinGraph Demo: Alzheimer's Disease Drug Discovery Knowledge Graph
-- ============================================================================
--
-- This demo showcases RaisinDB's graph capabilities for bioinformatics:
-- 1. GRAPH_TABLE queries (ISO SQL:2023 Part 16)
-- 2. Variable-length path traversal
-- 3. Vector embeddings for protein similarity
-- 4. PostgreSQL wire protocol (works with psql, Jupyter, DBeaver)
--
-- Data sources:
-- - Proteins: STRING database (string-db.org)
-- - Drugs: DrugBank
-- - Associations: DisGeNET
-- ============================================================================

-- ============================================================================
-- Schema: NodeType Definitions
-- ============================================================================

-- Drop existing types (ignore errors on first run)
DROP NODETYPE 'bio:Protein';
DROP NODETYPE 'bio:Drug';
DROP NODETYPE 'bio:Disease';

-- Protein node type with embedding support
CREATE NODETYPE 'bio:Protein' (
  PROPERTIES (
    gene_id String REQUIRED PROPERTY_INDEX LABEL 'Gene Symbol' ORDER 1,
    name String REQUIRED FULLTEXT LABEL 'Protein Name' ORDER 2,
    uniprot String PROPERTY_INDEX LABEL 'UniProt ID' ORDER 3,
    description String FULLTEXT LABEL 'Description' ORDER 4,
    molecular_weight Number LABEL 'Molecular Weight (Da)' ORDER 5,
    chromosome String PROPERTY_INDEX LABEL 'Chromosome' ORDER 6,
    organism String DEFAULT 'Homo sapiens' PROPERTY_INDEX LABEL 'Organism' ORDER 7,
    druggable Boolean DEFAULT false PROPERTY_INDEX LABEL 'Druggable Target' ORDER 8,
    embedding Array OF Number LABEL 'Protein Embedding (1536-dim)' ORDER 9
  )
  INDEXABLE
);

-- Drug node type
CREATE NODETYPE 'bio:Drug' (
  PROPERTIES (
    drug_id String REQUIRED PROPERTY_INDEX LABEL 'Drug ID' ORDER 1,
    name String REQUIRED FULLTEXT LABEL 'Drug Name' ORDER 2,
    brand_name String FULLTEXT LABEL 'Brand Name' ORDER 3,
    drugbank String PROPERTY_INDEX LABEL 'DrugBank ID' ORDER 4,
    mechanism String FULLTEXT LABEL 'Mechanism of Action' ORDER 5,
    fda_approved Boolean DEFAULT false PROPERTY_INDEX LABEL 'FDA Approved' ORDER 6,
    approval_year Number LABEL 'Approval Year' ORDER 7
  )
  INDEXABLE
);

-- Disease node type
CREATE NODETYPE 'bio:Disease' (
  PROPERTIES (
    disease_id String REQUIRED PROPERTY_INDEX LABEL 'Disease ID' ORDER 1,
    name String REQUIRED FULLTEXT LABEL 'Disease Name' ORDER 2,
    mesh String PROPERTY_INDEX LABEL 'MeSH ID' ORDER 3,
    description String FULLTEXT LABEL 'Description' ORDER 4
  )
  INDEXABLE
);

-- ============================================================================
-- Folder Structure
-- ============================================================================

-- Clean up existing data
DELETE FROM default WHERE path LIKE '/alzheimer-study/%';

-- Create root structure
UPSERT INTO default (path, node_type, name) VALUES ('/alzheimer-study', 'raisin:Folder', 'Alzheimer Study');
UPSERT INTO default (path, node_type, name) VALUES ('/alzheimer-study/proteins', 'raisin:Folder', 'Proteins');
UPSERT INTO default (path, node_type, name) VALUES ('/alzheimer-study/drugs', 'raisin:Folder', 'Drugs');
UPSERT INTO default (path, node_type, name) VALUES ('/alzheimer-study/diseases', 'raisin:Folder', 'Diseases');

-- ============================================================================
-- Proteins: Key Alzheimer's Disease Proteins
-- ============================================================================

-- Core amyloid pathway proteins
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/APP',
  'bio:Protein',
  'Amyloid Precursor Protein',
  '{
    "gene_id": "APP",
    "name": "Amyloid Precursor Protein",
    "uniprot": "P05067",
    "description": "Central protein in Alzheimer pathology. Cleaved to form amyloid-beta plaques.",
    "molecular_weight": 86943,
    "chromosome": "21",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/PSEN1',
  'bio:Protein',
  'Presenilin-1',
  '{
    "gene_id": "PSEN1",
    "name": "Presenilin-1",
    "uniprot": "P49768",
    "description": "Catalytic subunit of gamma-secretase. Mutations cause early-onset Alzheimer.",
    "molecular_weight": 52667,
    "chromosome": "14",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/PSEN2',
  'bio:Protein',
  'Presenilin-2',
  '{
    "gene_id": "PSEN2",
    "name": "Presenilin-2",
    "uniprot": "P49810",
    "description": "Gamma-secretase component. Less common cause of familial Alzheimer.",
    "molecular_weight": 50141,
    "chromosome": "1",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/BACE1',
  'bio:Protein',
  'Beta-secretase 1',
  '{
    "gene_id": "BACE1",
    "name": "Beta-secretase 1",
    "uniprot": "P56817",
    "description": "Cleaves APP to initiate amyloid-beta production. Major drug target.",
    "molecular_weight": 55821,
    "chromosome": "11",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/APOE',
  'bio:Protein',
  'Apolipoprotein E',
  '{
    "gene_id": "APOE",
    "name": "Apolipoprotein E",
    "uniprot": "P02649",
    "description": "Major genetic risk factor. APOE4 allele increases risk 3-15x.",
    "molecular_weight": 36154,
    "chromosome": "19",
    "organism": "Homo sapiens",
    "druggable": false
  }'::JSONB
);

-- Tau pathway proteins
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/MAPT',
  'bio:Protein',
  'Tau Protein',
  '{
    "gene_id": "MAPT",
    "name": "Microtubule-associated protein tau",
    "uniprot": "P10636",
    "description": "Forms neurofibrillary tangles in Alzheimer. Stabilizes microtubules.",
    "molecular_weight": 78928,
    "chromosome": "17",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/GSK3B',
  'bio:Protein',
  'GSK-3 Beta',
  '{
    "gene_id": "GSK3B",
    "name": "Glycogen synthase kinase 3 beta",
    "uniprot": "P49841",
    "description": "Key tau kinase. Phosphorylates tau leading to tangles.",
    "molecular_weight": 46744,
    "chromosome": "3",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/CDK5',
  'bio:Protein',
  'CDK5',
  '{
    "gene_id": "CDK5",
    "name": "Cyclin-dependent kinase 5",
    "uniprot": "Q00535",
    "description": "Neuronal kinase. Aberrant activation in Alzheimer.",
    "molecular_weight": 33688,
    "chromosome": "7",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

-- Gamma-secretase complex
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/NCSTN',
  'bio:Protein',
  'Nicastrin',
  '{
    "gene_id": "NCSTN",
    "name": "Nicastrin",
    "uniprot": "Q92542",
    "description": "Essential gamma-secretase component. Substrate receptor.",
    "molecular_weight": 78418,
    "chromosome": "1",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/APH1A',
  'bio:Protein',
  'APH-1A',
  '{
    "gene_id": "APH1A",
    "name": "Anterior pharynx-defective 1A",
    "uniprot": "Q96BI3",
    "description": "Gamma-secretase scaffolding protein. Assembly and stability.",
    "molecular_weight": 29156,
    "chromosome": "1",
    "organism": "Homo sapiens",
    "druggable": false
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/PEN2',
  'bio:Protein',
  'PEN-2',
  '{
    "gene_id": "PEN2",
    "name": "Presenilin enhancer 2",
    "uniprot": "Q9NZ42",
    "description": "Smallest gamma-secretase subunit. Required for complex maturation.",
    "molecular_weight": 12004,
    "chromosome": "19",
    "organism": "Homo sapiens",
    "druggable": false
  }'::JSONB
);

-- Secretases
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/ADAM10',
  'bio:Protein',
  'ADAM10',
  '{
    "gene_id": "ADAM10",
    "name": "Disintegrin and metalloproteinase 10",
    "uniprot": "O14672",
    "description": "Alpha-secretase that cleaves APP in non-amyloidogenic pathway.",
    "molecular_weight": 84141,
    "chromosome": "15",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

-- GWAS risk genes
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/TREM2',
  'bio:Protein',
  'TREM2',
  '{
    "gene_id": "TREM2",
    "name": "Triggering receptor expressed on myeloid cells 2",
    "uniprot": "Q9NZC2",
    "description": "Microglial receptor. Variants increase Alzheimer risk 2-4x.",
    "molecular_weight": 25578,
    "chromosome": "6",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/CLU',
  'bio:Protein',
  'Clusterin',
  '{
    "gene_id": "CLU",
    "name": "Clusterin",
    "uniprot": "P10909",
    "description": "Chaperone protein. GWAS-identified Alzheimer risk gene.",
    "molecular_weight": 52495,
    "chromosome": "8",
    "organism": "Homo sapiens",
    "druggable": false
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/BIN1',
  'bio:Protein',
  'BIN1',
  '{
    "gene_id": "BIN1",
    "name": "Bridging integrator 1",
    "uniprot": "O00499",
    "description": "Second strongest GWAS signal after APOE. Regulates endocytosis.",
    "molecular_weight": 64790,
    "chromosome": "2",
    "organism": "Homo sapiens",
    "druggable": false
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/SORL1',
  'bio:Protein',
  'SORL1',
  '{
    "gene_id": "SORL1",
    "name": "Sortilin-related receptor 1",
    "uniprot": "Q92673",
    "description": "Sorting receptor for APP. Directs APP away from amyloidogenic pathway.",
    "molecular_weight": 250353,
    "chromosome": "11",
    "organism": "Homo sapiens",
    "druggable": false
  }'::JSONB
);

-- Clearance mechanisms
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/LRP1',
  'bio:Protein',
  'LRP1',
  '{
    "gene_id": "LRP1",
    "name": "LDL receptor-related protein 1",
    "uniprot": "Q07954",
    "description": "Major APP and amyloid-beta receptor. Brain clearance.",
    "molecular_weight": 504605,
    "chromosome": "12",
    "organism": "Homo sapiens",
    "druggable": false
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/IDE',
  'bio:Protein',
  'IDE',
  '{
    "gene_id": "IDE",
    "name": "Insulin-degrading enzyme",
    "uniprot": "P14735",
    "description": "Degrades amyloid-beta and insulin. Clearance mechanism.",
    "molecular_weight": 117968,
    "chromosome": "10",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/NEP',
  'bio:Protein',
  'Neprilysin',
  '{
    "gene_id": "NEP",
    "name": "Neprilysin",
    "uniprot": "P08473",
    "description": "Major amyloid-beta degrading enzyme. Therapeutic target.",
    "molecular_weight": 85514,
    "chromosome": "3",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

-- Cholinergic system (current drug targets)
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/ACHE',
  'bio:Protein',
  'Acetylcholinesterase',
  '{
    "gene_id": "ACHE",
    "name": "Acetylcholinesterase",
    "uniprot": "P22303",
    "description": "Acetylcholine hydrolysis. Target of current AD drugs.",
    "molecular_weight": 67795,
    "chromosome": "7",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/NMDAR',
  'bio:Protein',
  'NMDA Receptor',
  '{
    "gene_id": "NMDAR",
    "name": "Glutamate receptor ionotropic NMDA",
    "uniprot": "Q05586",
    "description": "NMDA receptor. Excitotoxicity and memantine target.",
    "molecular_weight": 105442,
    "chromosome": "9",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

-- Neuroinflammation proteins
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/TNF',
  'bio:Protein',
  'TNF-alpha',
  '{
    "gene_id": "TNF",
    "name": "Tumor necrosis factor",
    "uniprot": "P01375",
    "description": "Pro-inflammatory cytokine. Neuroinflammation driver.",
    "molecular_weight": 25644,
    "chromosome": "6",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/IL1B',
  'bio:Protein',
  'Interleukin-1 beta',
  '{
    "gene_id": "IL1B",
    "name": "Interleukin-1 beta",
    "uniprot": "P01584",
    "description": "Pro-inflammatory cytokine. Microglial activation.",
    "molecular_weight": 30748,
    "chromosome": "2",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/IL6',
  'bio:Protein',
  'Interleukin-6',
  '{
    "gene_id": "IL6",
    "name": "Interleukin-6",
    "uniprot": "P05231",
    "description": "Pleiotropic cytokine. Elevated in AD patients.",
    "molecular_weight": 23718,
    "chromosome": "7",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/proteins/NFKB1',
  'bio:Protein',
  'NF-kB',
  '{
    "gene_id": "NFKB1",
    "name": "Nuclear factor NF-kappa-B",
    "uniprot": "P19838",
    "description": "Transcription factor. Inflammation master regulator.",
    "molecular_weight": 105356,
    "chromosome": "4",
    "organism": "Homo sapiens",
    "druggable": true
  }'::JSONB
);

-- ============================================================================
-- Drugs: Alzheimer's Disease Treatments
-- ============================================================================

-- Monoclonal antibodies (new treatments)
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/drugs/ADUCANUMAB',
  'bio:Drug',
  'Aducanumab',
  '{
    "drug_id": "ADUCANUMAB",
    "name": "Aducanumab",
    "brand_name": "Aduhelm",
    "drugbank": "DB16663",
    "mechanism": "Monoclonal antibody targeting aggregated amyloid-beta",
    "fda_approved": true,
    "approval_year": 2021
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/drugs/LECANEMAB',
  'bio:Drug',
  'Lecanemab',
  '{
    "drug_id": "LECANEMAB",
    "name": "Lecanemab",
    "brand_name": "Leqembi",
    "drugbank": "DB16678",
    "mechanism": "Monoclonal antibody targeting amyloid-beta protofibrils",
    "fda_approved": true,
    "approval_year": 2023
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/drugs/DONANEMAB',
  'bio:Drug',
  'Donanemab',
  '{
    "drug_id": "DONANEMAB",
    "name": "Donanemab",
    "brand_name": "Kisunla",
    "drugbank": "DB18310",
    "mechanism": "Monoclonal antibody targeting pyroglutamate amyloid-beta",
    "fda_approved": true,
    "approval_year": 2024
  }'::JSONB
);

-- Cholinesterase inhibitors (traditional treatments)
UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/drugs/DONEPEZIL',
  'bio:Drug',
  'Donepezil',
  '{
    "drug_id": "DONEPEZIL",
    "name": "Donepezil",
    "brand_name": "Aricept",
    "drugbank": "DB00843",
    "mechanism": "Acetylcholinesterase inhibitor",
    "fda_approved": true,
    "approval_year": 1996
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/drugs/RIVASTIGMINE',
  'bio:Drug',
  'Rivastigmine',
  '{
    "drug_id": "RIVASTIGMINE",
    "name": "Rivastigmine",
    "brand_name": "Exelon",
    "drugbank": "DB00989",
    "mechanism": "Acetylcholinesterase inhibitor",
    "fda_approved": true,
    "approval_year": 2000
  }'::JSONB
);

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/drugs/MEMANTINE',
  'bio:Drug',
  'Memantine',
  '{
    "drug_id": "MEMANTINE",
    "name": "Memantine",
    "brand_name": "Namenda",
    "drugbank": "DB01043",
    "mechanism": "NMDA receptor antagonist",
    "fda_approved": true,
    "approval_year": 2003
  }'::JSONB
);

-- ============================================================================
-- Diseases
-- ============================================================================

UPSERT INTO default (path, node_type, name, properties) VALUES (
  '/alzheimer-study/diseases/ALZHEIMER',
  'bio:Disease',
  'Alzheimer Disease',
  '{
    "disease_id": "ALZHEIMER",
    "name": "Alzheimer Disease",
    "mesh": "D000544",
    "description": "Progressive neurodegenerative disease characterized by amyloid plaques and neurofibrillary tangles"
  }'::JSONB
);

-- ============================================================================
-- Relationships: Protein-Protein Interactions (from STRING database)
-- ============================================================================

-- APP interactions (hub protein)
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/PSEN1' TYPE 'INTERACTS_WITH' WEIGHT 0.999;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/PSEN2' TYPE 'INTERACTS_WITH' WEIGHT 0.995;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/BACE1' TYPE 'INTERACTS_WITH' WEIGHT 0.998;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/ADAM10' TYPE 'INTERACTS_WITH' WEIGHT 0.994;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/APOE' TYPE 'INTERACTS_WITH' WEIGHT 0.987;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/LRP1' TYPE 'INTERACTS_WITH' WEIGHT 0.978;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/SORL1' TYPE 'INTERACTS_WITH' WEIGHT 0.956;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/CLU' TYPE 'INTERACTS_WITH' WEIGHT 0.934;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/MAPT' TYPE 'INTERACTS_WITH' WEIGHT 0.956;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/IDE' TYPE 'INTERACTS_WITH' WEIGHT 0.934;
RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/proteins/NEP' TYPE 'INTERACTS_WITH' WEIGHT 0.923;

-- Gamma-secretase complex (PSEN1, PSEN2, NCSTN, APH1A, PEN2)
RELATE FROM path='/alzheimer-study/proteins/PSEN1' TO path='/alzheimer-study/proteins/NCSTN' TYPE 'INTERACTS_WITH' WEIGHT 0.999;
RELATE FROM path='/alzheimer-study/proteins/PSEN1' TO path='/alzheimer-study/proteins/APH1A' TYPE 'INTERACTS_WITH' WEIGHT 0.998;
RELATE FROM path='/alzheimer-study/proteins/PSEN1' TO path='/alzheimer-study/proteins/PEN2' TYPE 'INTERACTS_WITH' WEIGHT 0.999;
RELATE FROM path='/alzheimer-study/proteins/PSEN1' TO path='/alzheimer-study/proteins/PSEN2' TYPE 'INTERACTS_WITH' WEIGHT 0.967;
RELATE FROM path='/alzheimer-study/proteins/PSEN2' TO path='/alzheimer-study/proteins/NCSTN' TYPE 'INTERACTS_WITH' WEIGHT 0.997;
RELATE FROM path='/alzheimer-study/proteins/PSEN2' TO path='/alzheimer-study/proteins/APH1A' TYPE 'INTERACTS_WITH' WEIGHT 0.993;
RELATE FROM path='/alzheimer-study/proteins/PSEN2' TO path='/alzheimer-study/proteins/PEN2' TYPE 'INTERACTS_WITH' WEIGHT 0.996;
RELATE FROM path='/alzheimer-study/proteins/NCSTN' TO path='/alzheimer-study/proteins/APH1A' TYPE 'INTERACTS_WITH' WEIGHT 0.995;
RELATE FROM path='/alzheimer-study/proteins/NCSTN' TO path='/alzheimer-study/proteins/PEN2' TYPE 'INTERACTS_WITH' WEIGHT 0.994;
RELATE FROM path='/alzheimer-study/proteins/APH1A' TO path='/alzheimer-study/proteins/PEN2' TYPE 'INTERACTS_WITH' WEIGHT 0.992;

-- Tau pathway (MAPT, GSK3B, CDK5)
RELATE FROM path='/alzheimer-study/proteins/MAPT' TO path='/alzheimer-study/proteins/GSK3B' TYPE 'INTERACTS_WITH' WEIGHT 0.998;
RELATE FROM path='/alzheimer-study/proteins/MAPT' TO path='/alzheimer-study/proteins/CDK5' TYPE 'INTERACTS_WITH' WEIGHT 0.994;
RELATE FROM path='/alzheimer-study/proteins/GSK3B' TO path='/alzheimer-study/proteins/CDK5' TYPE 'INTERACTS_WITH' WEIGHT 0.912;

-- APOE interactions
RELATE FROM path='/alzheimer-study/proteins/APOE' TO path='/alzheimer-study/proteins/LRP1' TYPE 'INTERACTS_WITH' WEIGHT 0.989;
RELATE FROM path='/alzheimer-study/proteins/APOE' TO path='/alzheimer-study/proteins/CLU' TYPE 'INTERACTS_WITH' WEIGHT 0.945;
RELATE FROM path='/alzheimer-study/proteins/APOE' TO path='/alzheimer-study/proteins/TREM2' TYPE 'INTERACTS_WITH' WEIGHT 0.912;

-- Microglial/immune interactions
RELATE FROM path='/alzheimer-study/proteins/TREM2' TO path='/alzheimer-study/proteins/APOE' TYPE 'INTERACTS_WITH' WEIGHT 0.912;

-- Neuroinflammation cascade
RELATE FROM path='/alzheimer-study/proteins/TNF' TO path='/alzheimer-study/proteins/IL1B' TYPE 'INTERACTS_WITH' WEIGHT 0.967;
RELATE FROM path='/alzheimer-study/proteins/TNF' TO path='/alzheimer-study/proteins/IL6' TYPE 'INTERACTS_WITH' WEIGHT 0.956;
RELATE FROM path='/alzheimer-study/proteins/TNF' TO path='/alzheimer-study/proteins/NFKB1' TYPE 'INTERACTS_WITH' WEIGHT 0.989;
RELATE FROM path='/alzheimer-study/proteins/IL1B' TO path='/alzheimer-study/proteins/IL6' TYPE 'INTERACTS_WITH' WEIGHT 0.978;
RELATE FROM path='/alzheimer-study/proteins/IL1B' TO path='/alzheimer-study/proteins/NFKB1' TYPE 'INTERACTS_WITH' WEIGHT 0.967;
RELATE FROM path='/alzheimer-study/proteins/IL6' TO path='/alzheimer-study/proteins/NFKB1' TYPE 'INTERACTS_WITH' WEIGHT 0.945;
RELATE FROM path='/alzheimer-study/proteins/GSK3B' TO path='/alzheimer-study/proteins/NFKB1' TYPE 'INTERACTS_WITH' WEIGHT 0.889;

-- GWAS risk gene interactions
RELATE FROM path='/alzheimer-study/proteins/BIN1' TO path='/alzheimer-study/proteins/CLU' TYPE 'INTERACTS_WITH' WEIGHT 0.756;
RELATE FROM path='/alzheimer-study/proteins/CLU' TO path='/alzheimer-study/proteins/LRP1' TYPE 'INTERACTS_WITH' WEIGHT 0.834;

-- ============================================================================
-- Relationships: Drug-Target Interactions
-- ============================================================================

-- Anti-amyloid antibodies target APP processing
RELATE FROM path='/alzheimer-study/drugs/ADUCANUMAB' TO path='/alzheimer-study/proteins/APP' TYPE 'TARGETS' WEIGHT 0.95;
RELATE FROM path='/alzheimer-study/drugs/LECANEMAB' TO path='/alzheimer-study/proteins/APP' TYPE 'TARGETS' WEIGHT 0.95;
RELATE FROM path='/alzheimer-study/drugs/DONANEMAB' TO path='/alzheimer-study/proteins/APP' TYPE 'TARGETS' WEIGHT 0.95;

-- Cholinesterase inhibitors
RELATE FROM path='/alzheimer-study/drugs/DONEPEZIL' TO path='/alzheimer-study/proteins/ACHE' TYPE 'TARGETS' WEIGHT 0.99;
RELATE FROM path='/alzheimer-study/drugs/RIVASTIGMINE' TO path='/alzheimer-study/proteins/ACHE' TYPE 'TARGETS' WEIGHT 0.99;

-- NMDA antagonist
RELATE FROM path='/alzheimer-study/drugs/MEMANTINE' TO path='/alzheimer-study/proteins/NMDAR' TYPE 'TARGETS' WEIGHT 0.99;

-- ============================================================================
-- Relationships: Disease Associations (from DisGeNET)
-- ============================================================================

RELATE FROM path='/alzheimer-study/proteins/APP' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 1.0;
RELATE FROM path='/alzheimer-study/proteins/PSEN1' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 1.0;
RELATE FROM path='/alzheimer-study/proteins/PSEN2' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 0.98;
RELATE FROM path='/alzheimer-study/proteins/APOE' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 1.0;
RELATE FROM path='/alzheimer-study/proteins/MAPT' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 0.96;
RELATE FROM path='/alzheimer-study/proteins/BACE1' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 0.94;
RELATE FROM path='/alzheimer-study/proteins/TREM2' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 0.92;
RELATE FROM path='/alzheimer-study/proteins/BIN1' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 0.89;
RELATE FROM path='/alzheimer-study/proteins/CLU' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 0.88;
RELATE FROM path='/alzheimer-study/proteins/SORL1' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 0.80;
RELATE FROM path='/alzheimer-study/proteins/ADAM10' TO path='/alzheimer-study/diseases/ALZHEIMER' TYPE 'ASSOCIATED_WITH' WEIGHT 0.78;

-- ============================================================================
-- Success message
-- ============================================================================
SELECT 'ProteinGraph demo data loaded successfully!' AS status,
       (SELECT COUNT(*) FROM default WHERE node_type = 'bio:Protein') AS proteins,
       (SELECT COUNT(*) FROM default WHERE node_type = 'bio:Drug') AS drugs,
       (SELECT COUNT(*) FROM default WHERE node_type = 'bio:Disease') AS diseases;
