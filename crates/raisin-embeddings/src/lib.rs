//! Vector embeddings support for RaisinDB.
//!
//! This crate provides the foundational infrastructure for managing vector embeddings
//! in RaisinDB, including:
//!
//! - Tenant-level embedding configuration
//! - Secure API key storage with AES-256-GCM encryption
//! - Storage abstractions for configuration persistence
//!
//! # Architecture
//!
//! The embeddings system is organized into several modules:
//!
//! - [`config`] - Configuration models for tenant embedding settings
//! - [`crypto`] - API key encryption/decryption using AES-256-GCM
//! - [`storage`] - Storage trait for configuration persistence
//!
//! # Usage
//!
//! ```rust,ignore
//! use raisin_embeddings::config::TenantEmbeddingConfig;
//! use raisin_embeddings::crypto::ApiKeyEncryptor;
//!
//! // Create a new tenant configuration
//! let mut config = TenantEmbeddingConfig::new("my-tenant".to_string());
//! config.enabled = true;
//!
//! // Encrypt API key
//! let master_key = [0u8; 32]; // Use secure key in production
//! let encryptor = ApiKeyEncryptor::new(&master_key);
//! let encrypted = encryptor.encrypt("sk-my-api-key").unwrap();
//! config.api_key_encrypted = Some(encrypted);
//!
//! // Store configuration (requires a storage implementation)
//! // store.set_config(&config).unwrap();
//! ```
//!
//! # Security Considerations
//!
//! - API keys are encrypted using AES-256-GCM before storage
//! - Master encryption keys should be stored securely (env vars, secrets manager)
//! - Never return encrypted API keys to clients
//! - Use separate storage from main data for enhanced security
//!
//! # Future Phases
//!
//! This is Phase 1.1 of the embeddings implementation. Future phases will add:
//!
//! - Phase 1.2: Vector storage column family and CRUD operations
//! - Phase 2: Embedding generation and background jobs
//! - Phase 3: Vector search capabilities
//! - Phase 4: API endpoints and client integration

pub mod config;
pub mod crypto;
pub mod embedding_storage;
pub mod models;
pub mod provider;
pub mod storage;

// Re-export commonly used types
pub use config::{EmbeddingProvider, TenantEmbeddingConfig};
pub use crypto::{ApiKeyEncryptor, CryptoError};
pub use embedding_storage::{EmbeddingJobStore, EmbeddingStorage};
pub use models::{EmbeddingData, EmbeddingJob, EmbeddingJobKind};
pub use provider::{create_provider, EmbeddingProvider as EmbeddingProviderTrait, OpenAIProvider};
pub use storage::{StorageError, TenantEmbeddingConfigStore};
