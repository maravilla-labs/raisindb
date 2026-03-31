Revised Full-Text Search Architecture (Final)This document outlines the complete, decoupled architecture for a persistent, language-aware, and crash-safe full-text search service. It incorporates all design decisions, including the raisin-indexer crate and the "one document per language" indexing strategy.1. Design PrinciplesAsync & Independent: Indexing happens asynchronously and does not block primary storage operations.Shared Abstraction: Core logic is defined by traits in raisin-storage, allowing for multiple implementations.Decoupled Indexing Service: A dedicated raisin-indexer crate contains the generic background worker, which is agnostic to the storage backend.Abstracted Job Persistence: A FullTextJobStore trait defines the contract for a persistent, crash-safe job queue.Branch-Centric Index Lifecycle: Each repository branch has its own independent full-text index.Language-Aware: The system indexes a separate, language-specific document for the default language and each available translation.Schema-Aware Indexing: Indexing logic uses NodeTypeSchema to determine which properties to include in the index.Revision-Aware: Every indexed document is tied to a specific node revision, ensuring consistency.2. Architectural OverviewThe architecture separates the application-facing API, the background indexing service, and the storage-specific implementation into distinct layers connected by traits. A persistent job queue guarantees that indexing tasks are not lost if the server restarts.Application (raisin-server): Interacts with the FullTextIndexRepository trait.raisin-storage: Defines all the abstract traits (FullTextIndexRepository, FullTextJobStore, IndexingEngine) and data structures (FullTextIndexJob).raisin-indexer: Contains the generic IndexerWorker that reads from the FullTextJobStore and writes to the IndexingEngine.raisin-rocksdb: Implements the FullTextJobStore and the IndexingEngine traits, using RocksDB for persistence and Tantivy for search.3. Layer 1: Abstraction Layer (raisin-storage)This crate defines the complete public contract for the full-text search feature.// In crates/raisin-storage/src/lib.rs

// --- High-level API, used by the application ---
pub trait FullTextIndexRepository: Send + Sync {
    // Enqueues a job to index a node.
    fn index_node(&self, job: FullTextIndexJob) -> impl Future<Output = Result<()>> + Send;
    // Enqueues a job to delete a node from the index.
    fn delete_node(&self, job: FullTextIndexJob) -> impl Future<Output = Result<()>> + Send;
    // Enqueues a job to handle branch creation.
    fn branch_created(&self, job: FullTextIndexJob) -> impl Future<Output = Result<()>> + Send;
    // Performs a search query against the index.
    fn search(&self, /* ... query parameters including language ... */) -> impl Future<Output = Result<Vec<String>>> + Send;
}

// --- Trait for the persistent job queue ---
pub trait FullTextJobStore: Send + Sync {
    fn enqueue(&self, job: &FullTextIndexJob) -> Result<()>;
    fn dequeue(&self, count: usize) -> Result<Vec<FullTextIndexJob>>;
    fn complete(&self, job_ids: &[String]) -> Result<()>;
    fn fail(&self, job_id: &str, error: &str) -> Result<()>;
}

// --- Trait for the actual indexing engine ---
pub trait IndexingEngine: Send + Sync {
    fn do_index_node(&self, job: &FullTextIndexJob) -> Result<()>;
    fn do_delete_node(&self, job: &FullTextIndexJob) -> Result<()>;
    fn do_branch_created(&self, job: &FullTextIndexJob) -> Result<()>;
}

// --- Concrete Job Definition ---
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum JobKind { AddNode, DeleteNode, BranchCreated }

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FullTextIndexJob {
    pub job_id: String, // Unique ID for this job
    pub kind: JobKind,
    pub tenant_id: String,
    pub repo_id: String,
    pub workspace_id: String,
    pub branch: String,
    pub revision: u64,
    pub node: Option<Node>,      // Present for AddNode
    pub node_id: Option<String>,   // Present for DeleteNode
    pub source_branch: Option<String>, // Present for BranchCreated
    pub workspace_config: WorkspaceConfig,
    pub node_type_schema: Option<NodeTypeSchema>,
}
4. Layer 2: Indexer Service (raisin-indexer)This crate provides the generic background worker. It has no knowledge of RocksDB or Tantivy.// In crates/raisin-indexer/src/worker.rs
pub struct IndexerWorker<S: FullTextJobStore, E: IndexingEngine> {
    job_store: Arc<S>,
    engine: Arc<E>,
}

impl<S, E> IndexerWorker<S, E> {
    // The main loop for the background thread.
    pub async fn run(&self) {
        loop {
            // 1. Dequeue jobs from the job_store.
            // 2. For each job, call the appropriate method on the engine.
            // 3. On success, mark jobs as complete in the job_store.
            // 4. On failure, mark job as failed and handle retries.
        }
    }
}
5. Layer 3: Storage & Engine Implementation (raisin-rocksdb)This crate provides the concrete, storage-specific implementations of the traits defined in raisin-storage.RocksDbJobStore: Implements FullTextJobStore using a dedicated RocksDB column family as a persistent queue.TantivyIndexingEngine: Implements IndexingEngine, containing all the logic for managing Tantivy indexes on disk, building schemas, and mapping Node structs to Tantivy documents.6. On-Disk Index StructureThe TantivyIndexingEngine will manage the full-text indexes in a structured directory hierarchy within the main data directory. This ensures isolation between tenants, repositories, and branches.<data_directory>/
└── fulltext/
    └── {tenant_id}/
        └── {repository_id}/
            ├── {branch_id_1}/  // Complete Tantivy index for this branch
            │   ├── meta.json
            │   ├── ... (other tantivy segment files)
            │   └── managed.json
            └── {branch_id_2}/  // Another independent Tantivy index
                ├── meta.json
                └── ...
Each {branch_id} directory is a self-contained Tantivy index.When a BranchCreated job is processed, the engine will copy the directory of the source_branch to create the new index.7. Layer 4: Server Wiring & Feature FlagsIn raisin-server, feature flags control which storage implementation is compiled and wired up at runtime.When the storage-rocksdb feature is enabled:RocksDbJobStore and TantivyIndexingEngine are instantiated.The IndexerWorker is created with these concrete implementations.The worker is started in a background tokio task.8. Layer 5: Workspace ConfigurationLanguage settings are defined in the WorkspaceConfig model.// In crates/raisin-models/src/workspaces.rs
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkspaceConfig {
    // ...
    pub default_language: String,
    pub supported_languages: Vec<String>,
}
9. Layer 6: Tantivy Schema & Data MappingThis section details the "one document per language" strategy implemented within the TantivyIndexingEngine.Schema DefinitionThe schema is static and language-agnostic.use tantivy::schema::*;
use tantivy_stemmers::{self, Language};

let mut schema_builder = Schema::builder();

// --- Core Identifiers and Metadata ---
schema_builder.add_text_field("doc_id", STRING | STORED); // Composite ID: node-branch-rev-lang
schema_builder.add_text_field("node_id", STRING | STORED);
schema_builder.add_text_field("workspace_id", STRING | INDEXED);
schema_builder.add_text_field("language", STRING | INDEXED);
schema_builder.add_text_field("path", STRING);
schema_builder.add_text_field("node_type", STRING);
schema_builder.add_u64_field("revision", U64 | INDEXED);
schema_builder.add_date_field("created_at", INDEXED | STORED);
schema_builder.add_date_field("updated_at", INDEXED | STORED);

// --- Static, Language-Analyzed Fields ---
let text_options = TextOptions::default() /* ... configured at index time ... */;
schema_builder.add_text_field("name", text_options.clone());
schema_builder.add_text_field("content", text_options);

let schema = schema_builder.build();
Data Mapping: Node -> Multiple Tantivy DocumentsA single Node is fanned out into multiple Tantivy documents—one for the default language and one for each translation.Process Default Language:Create a Tantivy Document.doc_id: f!("{node.id}-{job.branch}-{job.revision}-{default_lang}").Add fields: node_id, workspace_id, language (default), path, revision, etc.Add node.name to the name field.Flatten indexable node.properties into the content field.Add the document to the IndexWriter.Process Translations:Iterate through node.translations. For each language (e.g., "de"):Create a new Tantivy Document.doc_id: f!("{node.id}-{job.branch}-{job.revision}-de").Add fields, setting language to "de".Extract and add translated name and content.Add the document to the IndexWriter.Querying ImplicationsQueries are explicit and must include a filter for workspace_id and language.Search for "haus" in German: +workspace_id:my_ws +language:de +(name:haus OR content:haus)Search for "house" in English: +workspace_id:my_ws +language:en +(name:house OR content:house)