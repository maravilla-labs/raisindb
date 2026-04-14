use super::QueryEngine;
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use raisin_embeddings::config::{EmbeddingDistanceMetric, EmbeddingProvider};
use raisin_embeddings::crypto::ApiKeyEncryptor;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::ast::ai_config::{AIConfigOperation, AIConfigStatement, ConfigSetting};
use raisin_storage::Storage;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    pub(crate) async fn execute_ai_config(
        &self,
        stmt: &AIConfigStatement,
    ) -> Result<RowStream, Error> {
        tracing::info!("Executing AI config statement: {}", stmt.operation());

        match stmt {
            AIConfigStatement::ShowEmbeddingConfig => self.execute_show_embedding_config().await,
            AIConfigStatement::AlterEmbeddingConfig { settings } => {
                self.execute_alter_embedding_config(settings).await
            }
            AIConfigStatement::TestEmbeddingConnection => {
                self.execute_test_embedding_connection().await
            }
            AIConfigStatement::ShowAIProviders => self.execute_show_ai_providers().await,
            AIConfigStatement::ShowAIConfig => self.execute_show_ai_config().await,
            AIConfigStatement::AlterAIConfig { operation } => {
                self.execute_alter_ai_config(operation).await
            }
            AIConfigStatement::TestAIProvider { provider } => {
                self.execute_test_ai_provider(provider).await
            }
            AIConfigStatement::RebuildVectorIndex => self.execute_rebuild_vector_index().await,
            AIConfigStatement::RegenerateEmbeddings => self.execute_regenerate_embeddings().await,
            AIConfigStatement::ShowVectorIndexHealth => {
                self.execute_show_vector_index_health().await
            }
            AIConfigStatement::VerifyVectorIndex => self.execute_verify_vector_index().await,
        }
    }

    async fn execute_show_embedding_config(&self) -> Result<RowStream, Error> {
        let store = self
            .embedding_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("Embedding config store not available".to_string()))?;

        let config = store
            .get_config(&self.tenant_id)
            .map_err(|e| Error::Backend(format!("Failed to read embedding config: {}", e)))?;

        let config = config.unwrap_or_else(|| {
            raisin_embeddings::TenantEmbeddingConfig::new(self.tenant_id.clone())
        });

        let has_api_key = config.api_key_encrypted.is_some();

        let rows = vec![
            config_row("enabled", &config.enabled.to_string()),
            config_row("provider", &format!("{:?}", config.provider)),
            config_row("model", &config.model),
            config_row("dimensions", &config.dimensions.to_string()),
            config_row("has_api_key", &has_api_key.to_string()),
            config_row("base_url", config.base_url.as_deref().unwrap_or("")),
            config_row("include_name", &config.include_name.to_string()),
            config_row("include_path", &config.include_path.to_string()),
            config_row(
                "default_max_distance",
                &config
                    .default_max_distance
                    .map(|d| format!("{:.2}", d))
                    .unwrap_or_else(|| "0.60 (default)".to_string()),
            ),
            config_row("distance_metric", &format!("{:?}", config.distance_metric)),
            config_row(
                "max_embeddings_per_repo",
                &config
                    .max_embeddings_per_repo
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unlimited".to_string()),
            ),
        ];

        ai_config_result_rows(rows)
    }

    async fn execute_alter_embedding_config(
        &self,
        settings: &[ConfigSetting],
    ) -> Result<RowStream, Error> {
        let store = self
            .embedding_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("Embedding config store not available".to_string()))?;

        let mut config = store
            .get_config(&self.tenant_id)
            .map_err(|e| Error::Backend(format!("Failed to read embedding config: {}", e)))?
            .unwrap_or_else(|| {
                raisin_embeddings::TenantEmbeddingConfig::new(self.tenant_id.clone())
            });

        for setting in settings {
            match setting.key.to_uppercase().as_str() {
                "PROVIDER" => {
                    config.provider = parse_provider(&setting.value)?;
                }
                "MODEL" => {
                    config.model = setting.value.clone();
                }
                "DIMENSIONS" => {
                    config.dimensions = setting.value.parse::<usize>().map_err(|_| {
                        Error::Validation(format!(
                            "Invalid dimensions value '{}': expected integer",
                            setting.value
                        ))
                    })?;
                }
                "API_KEY" => {
                    let master_key = self.master_key.as_ref().ok_or_else(|| {
                        Error::Validation(
                            "Master key not configured, cannot encrypt API key".to_string(),
                        )
                    })?;
                    let encryptor = ApiKeyEncryptor::new(master_key);
                    let encrypted = encryptor
                        .encrypt(&setting.value)
                        .map_err(|e| Error::Backend(format!("Failed to encrypt API key: {}", e)))?;
                    config.api_key_encrypted = Some(encrypted);
                }
                "BASE_URL" => {
                    config.base_url = if setting.value.is_empty() {
                        None
                    } else {
                        Some(setting.value.clone())
                    };
                }
                "ENABLED" => {
                    config.enabled = parse_bool(&setting.value).map_err(|_| {
                        Error::Validation(format!(
                            "Invalid enabled value '{}': expected 'true' or 'false'",
                            setting.value
                        ))
                    })?;
                }
                "INCLUDE_NAME" => {
                    config.include_name = parse_bool(&setting.value).map_err(|_| {
                        Error::Validation(format!(
                            "Invalid include_name value '{}': expected 'true' or 'false'",
                            setting.value
                        ))
                    })?;
                }
                "INCLUDE_PATH" => {
                    config.include_path = parse_bool(&setting.value).map_err(|_| {
                        Error::Validation(format!(
                            "Invalid include_path value '{}': expected 'true' or 'false'",
                            setting.value
                        ))
                    })?;
                }
                "DEFAULT_MAX_DISTANCE" => {
                    config.default_max_distance = if setting.value.to_lowercase() == "none"
                        || setting.value.to_lowercase() == "default"
                    {
                        None
                    } else {
                        Some(setting.value.parse::<f32>().map_err(|_| {
                            Error::Validation(format!(
                                "Invalid default_max_distance value '{}': expected float (e.g., 0.5)",
                                setting.value
                            ))
                        })?)
                    };
                }
                "DISTANCE_METRIC" => {
                    config.distance_metric = parse_distance_metric(&setting.value)?;
                }
                "MAX_EMBEDDINGS_PER_REPO" => {
                    config.max_embeddings_per_repo = if setting.value.to_lowercase() == "unlimited"
                        || setting.value == "0"
                    {
                        None
                    } else {
                        Some(setting.value.parse::<usize>().map_err(|_| {
                            Error::Validation(format!(
                                "Invalid max_embeddings_per_repo value '{}': expected integer or 'unlimited'",
                                setting.value
                            ))
                        })?)
                    };
                }
                other => {
                    return Err(Error::Validation(format!(
                        "Unknown embedding config setting: '{}'",
                        other
                    )));
                }
            }
        }

        store
            .set_config(&config)
            .map_err(|e| Error::Backend(format!("Failed to save embedding config: {}", e)))?;

        ai_config_ok("Embedding configuration updated")
    }

    async fn execute_test_embedding_connection(&self) -> Result<RowStream, Error> {
        let store = self
            .embedding_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("Embedding config store not available".to_string()))?;

        let config = store
            .get_config(&self.tenant_id)
            .map_err(|e| Error::Backend(format!("Failed to read embedding config: {}", e)))?
            .ok_or_else(|| {
                Error::Validation("No embedding configuration found for this tenant".to_string())
            })?;

        let api_key = self.decrypt_api_key(&config)?;

        let provider = raisin_embeddings::create_provider_with_url(
            &config.provider,
            &api_key,
            &config.model,
            config.base_url.as_deref(),
        )
        .map_err(|e| Error::Backend(format!("Failed to create embedding provider: {}", e)))?;

        match provider.test_connection().await {
            Ok(dimensions) => {
                let mut row = Row::new();
                row.insert(
                    "result".to_string(),
                    PropertyValue::String("Connection successful".to_string()),
                );
                row.insert(
                    "dimensions".to_string(),
                    PropertyValue::Integer(dimensions as i64),
                );
                row.insert(
                    "model".to_string(),
                    PropertyValue::String(config.model.clone()),
                );
                row.insert("success".to_string(), PropertyValue::Boolean(true));
                Ok(Box::pin(stream::once(async move { Ok(row) })))
            }
            Err(e) => {
                let mut row = Row::new();
                row.insert(
                    "result".to_string(),
                    PropertyValue::String(format!("Connection failed: {}", e)),
                );
                row.insert("success".to_string(), PropertyValue::Boolean(false));
                Ok(Box::pin(stream::once(async move { Ok(row) })))
            }
        }
    }

    async fn execute_show_ai_providers(&self) -> Result<RowStream, Error> {
        let store = self
            .embedding_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("Embedding config store not available".to_string()))?;

        let config = store
            .get_config(&self.tenant_id)
            .map_err(|e| Error::Backend(format!("Failed to read embedding config: {}", e)))?;

        match config {
            Some(cfg) => {
                let mut row = Row::new();
                row.insert(
                    "provider".to_string(),
                    PropertyValue::String(format!("{:?}", cfg.provider)),
                );
                row.insert("model".to_string(), PropertyValue::String(cfg.model));
                row.insert("enabled".to_string(), PropertyValue::Boolean(cfg.enabled));
                row.insert(
                    "has_api_key".to_string(),
                    PropertyValue::Boolean(cfg.api_key_encrypted.is_some()),
                );
                ai_config_result_rows(vec![row])
            }
            None => ai_config_result_rows(vec![]),
        }
    }

    async fn execute_show_ai_config(&self) -> Result<RowStream, Error> {
        self.execute_show_embedding_config().await
    }

    async fn execute_alter_ai_config(
        &self,
        operation: &AIConfigOperation,
    ) -> Result<RowStream, Error> {
        let store = self
            .embedding_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("Embedding config store not available".to_string()))?;

        let mut config = store
            .get_config(&self.tenant_id)
            .map_err(|e| Error::Backend(format!("Failed to read embedding config: {}", e)))?
            .unwrap_or_else(|| {
                raisin_embeddings::TenantEmbeddingConfig::new(self.tenant_id.clone())
            });

        match operation {
            AIConfigOperation::AddProvider { provider, settings } => {
                config.provider = parse_provider(provider)?;
                for setting in settings {
                    match setting.key.to_uppercase().as_str() {
                        "MODEL" => config.model = setting.value.clone(),
                        "API_KEY" => {
                            let master_key = self.master_key.as_ref().ok_or_else(|| {
                                Error::Validation(
                                    "Master key not configured, cannot encrypt API key".to_string(),
                                )
                            })?;
                            let encryptor = ApiKeyEncryptor::new(master_key);
                            let encrypted = encryptor.encrypt(&setting.value).map_err(|e| {
                                Error::Backend(format!("Failed to encrypt API key: {}", e))
                            })?;
                            config.api_key_encrypted = Some(encrypted);
                        }
                        "BASE_URL" => {
                            config.base_url = if setting.value.is_empty() {
                                None
                            } else {
                                Some(setting.value.clone())
                            };
                        }
                        "DIMENSIONS" => {
                            config.dimensions = setting.value.parse::<usize>().map_err(|_| {
                                Error::Validation(format!(
                                    "Invalid dimensions value '{}': expected integer",
                                    setting.value
                                ))
                            })?;
                        }
                        other => {
                            return Err(Error::Validation(format!(
                                "Unknown provider setting: '{}'",
                                other
                            )));
                        }
                    }
                }
                config.enabled = true;

                store.set_config(&config).map_err(|e| {
                    Error::Backend(format!("Failed to save embedding config: {}", e))
                })?;

                ai_config_ok(format!("Provider '{}' configured and enabled", provider))
            }
            AIConfigOperation::DropProvider { provider } => {
                let current = format!("{:?}", config.provider);
                if current.to_uppercase() != provider.to_uppercase() {
                    return Err(Error::Validation(format!(
                        "Provider '{}' is not configured (current: {})",
                        provider, current
                    )));
                }
                config.enabled = false;

                store.set_config(&config).map_err(|e| {
                    Error::Backend(format!("Failed to save embedding config: {}", e))
                })?;

                ai_config_ok(format!("Provider '{}' disabled", provider))
            }
        }
    }

    async fn execute_test_ai_provider(&self, _provider: &str) -> Result<RowStream, Error> {
        self.execute_test_embedding_connection().await
    }

    async fn execute_rebuild_vector_index(&self) -> Result<RowStream, Error> {
        let engine = self
            .hnsw_engine
            .as_ref()
            .ok_or_else(|| Error::Validation("HNSW engine not configured".to_string()))?;

        let branch = self.effective_branch().await;

        // Purge existing index
        engine
            .purge_index(&self.tenant_id, &self.repo_id, &branch, "default")
            .map_err(|e| Error::Backend(format!("Failed to purge vector index: {}", e)))?;

        // Get embedding config for dimensions
        let store = self
            .embedding_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("Embedding config store not available".to_string()))?;
        let config = store
            .get_config(&self.tenant_id)
            .map_err(|e| Error::Backend(format!("Failed to read embedding config: {}", e)))?
            .unwrap_or_else(|| {
                raisin_embeddings::TenantEmbeddingConfig::new(self.tenant_id.clone())
            });

        // Create fresh index with configured dimensions
        engine
            .create_index_with_dimensions(
                &self.tenant_id,
                &self.repo_id,
                &branch,
                config.dimensions,
            )
            .map_err(|e| Error::Backend(format!("Failed to create new index: {}", e)))?;

        // Re-populate from embedding storage
        if let Some(ref emb_storage) = self.embedding_storage {
            let embeddings = emb_storage
                .list_embeddings(&self.tenant_id, &self.repo_id, &branch, "default")
                .map_err(|e| Error::Backend(format!("Failed to list embeddings: {}", e)))?;

            let count = embeddings.len();
            for (node_id, revision) in &embeddings {
                if let Ok(Some(data)) = emb_storage.get_embedding(
                    &self.tenant_id,
                    &self.repo_id,
                    &branch,
                    "default",
                    node_id,
                    Some(revision),
                ) {
                    let _ = engine.add_embedding(
                        &self.tenant_id,
                        &self.repo_id,
                        &branch,
                        "default",
                        node_id,
                        *revision,
                        data.vector,
                    );
                }
            }

            ai_config_ok(format!("Vector index rebuilt with {} embeddings", count))
        } else {
            ai_config_ok("Vector index purged. No embedding storage available to repopulate.")
        }
    }

    async fn execute_regenerate_embeddings(&self) -> Result<RowStream, Error> {
        let _engine = self
            .hnsw_engine
            .as_ref()
            .ok_or_else(|| Error::Validation("HNSW engine not configured".to_string()))?;

        let store = self
            .embedding_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("Embedding config store not available".to_string()))?;

        let config = store
            .get_config(&self.tenant_id)
            .map_err(|e| Error::Backend(format!("Failed to read embedding config: {}", e)))?;

        if config.is_none() || !config.as_ref().unwrap().enabled {
            return Err(Error::Validation(
                "Embeddings not enabled for this tenant. Configure with ALTER EMBEDDING CONFIG first.".to_string(),
            ));
        }

        // Count existing embeddings to give user feedback
        let branch = self.effective_branch().await;
        let count = if let Some(ref emb_storage) = self.embedding_storage {
            emb_storage
                .list_embeddings(&self.tenant_id, &self.repo_id, &branch, "default")
                .map(|list| list.len())
                .unwrap_or(0)
        } else {
            0
        };

        ai_config_ok(format!(
            "Embedding regeneration requires the background worker. \
             Current index has {} embeddings. \
             To regenerate, use the REST API: POST /api/admin/management/database/{}/{}/vector/regenerate",
            count, self.tenant_id, self.repo_id
        ))
    }

    async fn execute_show_vector_index_health(&self) -> Result<RowStream, Error> {
        if let Some(ref engine) = self.hnsw_engine {
            let branch = self.effective_branch().await;
            match engine.stats(&self.tenant_id, &self.repo_id, &branch) {
                Ok(stats) => {
                    let mut row = Row::new();
                    row.insert(
                        "status".to_string(),
                        PropertyValue::String("available".to_string()),
                    );
                    row.insert(
                        "count".to_string(),
                        PropertyValue::Integer(stats.count as i64),
                    );
                    row.insert(
                        "dimensions".to_string(),
                        PropertyValue::Integer(stats.dimensions as i64),
                    );
                    row.insert(
                        "memory_bytes".to_string(),
                        PropertyValue::Integer(stats.memory_bytes as i64),
                    );
                    ai_config_result_rows(vec![row])
                }
                Err(e) => {
                    let mut row = Row::new();
                    row.insert(
                        "status".to_string(),
                        PropertyValue::String("error".to_string()),
                    );
                    row.insert(
                        "details".to_string(),
                        PropertyValue::String(format!("{}", e)),
                    );
                    ai_config_result_rows(vec![row])
                }
            }
        } else {
            let mut row = Row::new();
            row.insert(
                "status".to_string(),
                PropertyValue::String("unavailable".to_string()),
            );
            row.insert(
                "details".to_string(),
                PropertyValue::String("HNSW engine not configured".to_string()),
            );
            ai_config_result_rows(vec![row])
        }
    }

    async fn execute_verify_vector_index(&self) -> Result<RowStream, Error> {
        let engine = self
            .hnsw_engine
            .as_ref()
            .ok_or_else(|| Error::Validation("HNSW engine not configured".to_string()))?;

        let branch = self.effective_branch().await;

        // Get HNSW index count
        let hnsw_count = match engine.stats(&self.tenant_id, &self.repo_id, &branch) {
            Ok(stats) => stats.count,
            Err(_) => 0,
        };

        // Get embedding storage count
        let storage_count = if let Some(ref emb_storage) = self.embedding_storage {
            emb_storage
                .list_embeddings(&self.tenant_id, &self.repo_id, &branch, "default")
                .map(|list| list.len())
                .unwrap_or(0)
        } else {
            0
        };

        let is_consistent = hnsw_count == storage_count;
        let status = if is_consistent {
            "consistent"
        } else {
            "mismatch"
        };

        let mut row = Row::new();
        row.insert(
            "status".to_string(),
            PropertyValue::String(status.to_string()),
        );
        row.insert(
            "hnsw_count".to_string(),
            PropertyValue::Integer(hnsw_count as i64),
        );
        row.insert(
            "storage_count".to_string(),
            PropertyValue::Integer(storage_count as i64),
        );
        if !is_consistent {
            row.insert(
                "action".to_string(),
                PropertyValue::String("Run REBUILD VECTOR INDEX to fix".to_string()),
            );
        }

        ai_config_result_rows(vec![row])
    }

    fn decrypt_api_key(
        &self,
        config: &raisin_embeddings::TenantEmbeddingConfig,
    ) -> Result<String, Error> {
        let encrypted = config.api_key_encrypted.as_ref().ok_or_else(|| {
            Error::Validation("No API key configured for this tenant".to_string())
        })?;

        let master_key = self.master_key.as_ref().ok_or_else(|| {
            Error::Validation("Master key not configured, cannot decrypt API key".to_string())
        })?;

        let encryptor = ApiKeyEncryptor::new(master_key);
        encryptor
            .decrypt(encrypted)
            .map_err(|e| Error::Backend(format!("Failed to decrypt API key: {}", e)))
    }
}

fn config_row(key: &str, value: &str) -> Row {
    let mut row = Row::new();
    row.insert("key".to_string(), PropertyValue::String(key.to_string()));
    row.insert(
        "value".to_string(),
        PropertyValue::String(value.to_string()),
    );
    row
}

fn ai_config_ok(message: impl Into<String>) -> Result<RowStream, Error> {
    let mut row = Row::new();
    row.insert("result".to_string(), PropertyValue::String(message.into()));
    row.insert("success".to_string(), PropertyValue::Boolean(true));
    Ok(Box::pin(stream::once(async move { Ok(row) })))
}

fn ai_config_result_rows(rows: Vec<Row>) -> Result<RowStream, Error> {
    let results: Vec<Result<Row, Error>> = rows.into_iter().map(Ok).collect();
    Ok(Box::pin(stream::iter(results)))
}

fn parse_provider(value: &str) -> Result<EmbeddingProvider, Error> {
    match value.to_uppercase().as_str() {
        "OPENAI" => Ok(EmbeddingProvider::OpenAI),
        "CLAUDE" | "VOYAGE" => Ok(EmbeddingProvider::Claude),
        "OLLAMA" => Ok(EmbeddingProvider::Ollama),
        "HUGGINGFACE" | "HUGGING_FACE" => Ok(EmbeddingProvider::HuggingFace),
        other => Err(Error::Validation(format!(
            "Unknown embedding provider '{}'. Supported: OpenAI, Claude, Ollama, HuggingFace",
            other
        ))),
    }
}

fn parse_bool(value: &str) -> Result<bool, ()> {
    match value.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(()),
    }
}

fn parse_distance_metric(value: &str) -> Result<EmbeddingDistanceMetric, Error> {
    match value.to_uppercase().as_str() {
        "COSINE" => Ok(EmbeddingDistanceMetric::Cosine),
        "L2" | "EUCLIDEAN" => Ok(EmbeddingDistanceMetric::L2),
        "INNER_PRODUCT" | "INNERPRODUCT" | "IP" => Ok(EmbeddingDistanceMetric::InnerProduct),
        "HAMMING" => Ok(EmbeddingDistanceMetric::Hamming),
        other => Err(Error::Validation(format!(
            "Unknown distance metric '{}'. Supported: Cosine (recommended), L2, InnerProduct, Hamming",
            other
        ))),
    }
}
