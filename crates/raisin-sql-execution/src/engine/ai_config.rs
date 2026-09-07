use super::QueryEngine;
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use raisin_embeddings::config::{EmbeddingDistanceMetric, EmbeddingProvider};
use raisin_embeddings::crypto::ApiKeyEncryptor;
use raisin_embeddings::resolve::{
    AIConfigStorageError, AIModelConfig, AIProviderConfig, AIProviderKind, AIUseCase,
    TenantAIConfig,
};
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

        // Resolution goes through the ONE resolver, the same one the embedding
        // job handler uses. That is the whole point of this statement: a green
        // "Connection successful" here must mean the job will succeed. It used
        // to demand an API key unconditionally and ignore `ai_provider_ref`
        // entirely, so it disagreed with the job in both directions — a
        // keyless Ollama config was rejected here and worked there, and a
        // console-configured unified ref was tested against the stale legacy
        // fields.
        let master_key = self.master_key.as_ref().ok_or_else(|| {
            Error::Validation("Master key not configured, cannot resolve provider".to_string())
        })?;

        let ai_config = if config.uses_unified_provider() {
            let store = self.ai_config_store.as_ref().ok_or_else(|| {
                Error::Validation("AI provider config store not available".to_string())
            })?;
            Some(
                store
                    .get_config(&self.tenant_id)
                    .await
                    .map_err(|e| Error::Backend(format!("Failed to read AI config: {}", e)))?,
            )
        } else {
            None
        };

        // `resolve_settings` first, so the row can name the model the job will
        // actually request. `config.model` is stale by construction under a
        // unified `ai_provider_ref`, so reporting it would have this statement
        // announce a successful test against a model it never called.
        let resolved =
            match raisin_embeddings::resolve_settings(&config, ai_config.as_ref(), master_key) {
                Ok(r) => r,
                // A resolution failure IS the test result — reporting it as a
                // statement error would hide exactly the misconfiguration this
                // statement exists to surface.
                Err(e) => {
                    let mut row = Row::new();
                    row.insert(
                        "result".to_string(),
                        PropertyValue::String(format!("Connection failed: {}", e)),
                    );
                    row.insert(
                        "model".to_string(),
                        PropertyValue::String(config.model.clone()),
                    );
                    row.insert("success".to_string(), PropertyValue::Boolean(false));
                    return Ok(Box::pin(stream::once(async move { Ok(row) })));
                }
            };

        let model = resolved.model.clone();
        let provider = match resolved.build() {
            Ok(p) => p,
            Err(e) => {
                let mut row = Row::new();
                row.insert(
                    "result".to_string(),
                    PropertyValue::String(format!("Connection failed: {}", e)),
                );
                row.insert("model".to_string(), PropertyValue::String(model));
                row.insert("success".to_string(), PropertyValue::Boolean(false));
                return Ok(Box::pin(stream::once(async move { Ok(row) })));
            }
        };

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
                row.insert("model".to_string(), PropertyValue::String(model));
                row.insert("success".to_string(), PropertyValue::Boolean(true));
                Ok(Box::pin(stream::once(async move { Ok(row) })))
            }
            Err(e) => {
                let mut row = Row::new();
                row.insert(
                    "result".to_string(),
                    PropertyValue::String(format!("Connection failed: {}", e)),
                );
                row.insert("model".to_string(), PropertyValue::String(model));
                row.insert("success".to_string(), PropertyValue::Boolean(false));
                Ok(Box::pin(stream::once(async move { Ok(row) })))
            }
        }
    }

    /// `SHOW AI PROVIDERS` / `SHOW AI CONFIG`: one row per provider in the
    /// tenant's provider list (the same record the HTTP `/ai/config` endpoint
    /// and the CLI edit). API keys are never shown; `has_api_key` is.
    async fn execute_show_ai_providers(&self) -> Result<RowStream, Error> {
        let config = self.load_tenant_ai_config().await?;
        let rows = config.providers.iter().map(provider_row).collect();
        ai_config_result_rows(rows)
    }

    async fn execute_show_ai_config(&self) -> Result<RowStream, Error> {
        self.execute_show_ai_providers().await
    }

    /// `ALTER AI CONFIG ADD PROVIDER '<slug>' [SET ...]` and
    /// `ALTER AI CONFIG DROP PROVIDER '<slug>'` edit the tenant's provider
    /// list. The embedding configuration is a separate record with its own
    /// statement (`ALTER EMBEDDING CONFIG`).
    async fn execute_alter_ai_config(
        &self,
        operation: &AIConfigOperation,
    ) -> Result<RowStream, Error> {
        let store = self
            .ai_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("AI config store not available".to_string()))?;

        let mut config = self.load_tenant_ai_config().await?;

        let encrypt = |plain: &str| -> Result<Vec<u8>, Error> {
            let master_key = self.master_key.as_ref().ok_or_else(|| {
                Error::Validation("Master key not configured, cannot encrypt API key".to_string())
            })?;
            ApiKeyEncryptor::new(master_key)
                .encrypt(plain)
                .map_err(|e| Error::Backend(format!("Failed to encrypt API key: {}", e)))
        };

        let message = apply_provider_operation(&mut config, operation, encrypt)?;

        store
            .set_config(&config)
            .await
            .map_err(|e| Error::Backend(format!("Failed to save AI config: {}", e)))?;

        ai_config_ok(message)
    }

    /// The tenant's provider list, or an empty one when nothing is stored yet.
    /// A read error is an error: merging onto an empty list would write it
    /// back and drop every provider the caller did not name.
    async fn load_tenant_ai_config(&self) -> Result<TenantAIConfig, Error> {
        let store = self
            .ai_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("AI config store not available".to_string()))?;

        match store.get_config(&self.tenant_id).await {
            Ok(config) => Ok(config),
            Err(AIConfigStorageError::NotFound(_)) => {
                Ok(TenantAIConfig::new(self.tenant_id.clone()))
            }
            Err(e) => Err(Error::Backend(format!("Failed to read AI config: {}", e))),
        }
    }

    async fn execute_test_ai_provider(&self, _provider: &str) -> Result<RowStream, Error> {
        self.execute_test_embedding_connection().await
    }

    /// `REBUILD VECTOR INDEX`
    ///
    /// Delegates to `HnswManagement::rebuild_index` — the SAME implementation
    /// the HTTP management endpoint uses. This used to be a second, drifted
    /// copy of that loop which:
    ///   * hardcoded the workspace to `"default"` (management hardcoded
    ///     `"staff"`), so it rebuilt nothing for content living anywhere else,
    ///     while the embedding job indexes whatever workspace the node is in;
    ///   * never compared a stored vector's width to the configured one, so a
    ///     width change silently produced an index the engine then rejected;
    ///   * discarded both the fetch error and the insert error (`if let Ok`,
    ///     `let _ =`); and
    ///   * reported the number of embeddings LISTED, not added — which is how
    ///     "Vector index rebuilt with 6 embeddings" could sit next to
    ///     `SHOW VECTOR INDEX HEALTH -> count: 0`.
    async fn execute_rebuild_vector_index(&self) -> Result<RowStream, Error> {
        let engine = self
            .hnsw_engine
            .as_ref()
            .ok_or_else(|| Error::Validation("HNSW engine not configured".to_string()))?;

        let config_store = self
            .embedding_config_store
            .as_ref()
            .ok_or_else(|| Error::Validation("Embedding config store not available".to_string()))?;

        let Some(ref emb_storage) = self.embedding_storage else {
            return ai_config_ok(
                "Vector index not rebuilt: no embedding storage available to read from.",
            );
        };

        let branch = self.effective_branch().await;

        let management = raisin_rocksdb::HnswManagement::from_stores(
            engine.clone(),
            emb_storage.clone(),
            config_store.clone(),
        );

        let stats = management
            .rebuild_index(&self.tenant_id, &self.repo_id, &branch, None)
            .await
            .map_err(|e| Error::Backend(format!("Failed to rebuild vector index: {}", e)))?;

        let where_ = if stats.workspaces.is_empty() {
            "no workspaces hold embeddings".to_string()
        } else {
            format!("workspaces: {}", stats.workspaces.join(", "))
        };

        if stats.errors > 0 {
            ai_config_ok(format!(
                "Vector index rebuilt: {} embeddings indexed, {} skipped ({})",
                stats.items_processed, stats.errors, where_
            ))
        } else {
            ai_config_ok(format!(
                "Vector index rebuilt: {} embeddings indexed ({})",
                stats.items_processed, where_
            ))
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

    /// `SHOW VECTOR INDEX HEALTH` — one row per PARTITION.
    ///
    /// A branch holds one index per embedding space (`{embedder_hash}{kind}`),
    /// so a single-row answer could only ever describe one of them, and an
    /// operator cannot rebuild a partition they cannot see. The `partition`
    /// column is the file stem on disk, so a row here names the thing
    /// `REBUILD VECTOR INDEX` acts on.
    ///
    /// `quantization` and `metric` are the ones the graph was BUILT with, read
    /// out of its `.hnsw.meta` sidecar — not the tenant's current config. That
    /// distinction is the point: an index keeps the shape it was written with,
    /// and comparing these two columns against the config is how an operator
    /// finds out a setting has not taken effect yet.
    async fn execute_show_vector_index_health(&self) -> Result<RowStream, Error> {
        let Some(ref engine) = self.hnsw_engine else {
            let mut row = Row::new();
            row.insert(
                "status".to_string(),
                PropertyValue::String("unavailable".to_string()),
            );
            row.insert(
                "details".to_string(),
                PropertyValue::String("HNSW engine not configured".to_string()),
            );
            return ai_config_result_rows(vec![row]);
        };

        let branch = self.effective_branch().await;
        let configured = engine.default_text_partition(&self.tenant_id, &self.repo_id, &branch);

        let partitions = match engine.list_partitions(&self.tenant_id, &self.repo_id, &branch) {
            Ok(p) => p,
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
                return ai_config_result_rows(vec![row]);
            }
        };

        // No file on disk yet is not an error — it is what a branch that has
        // never been embedded looks like. Report the partition the tenant WOULD
        // write to, so the operator sees the identity even before the first
        // vector exists.
        if partitions.is_empty() {
            let mut row = Row::new();
            row.insert(
                "status".to_string(),
                PropertyValue::String("empty".to_string()),
            );
            row.insert(
                "partition".to_string(),
                PropertyValue::String(
                    configured
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "(unresolved)".to_string()),
                ),
            );
            row.insert("count".to_string(), PropertyValue::Integer(0));
            row.insert(
                "details".to_string(),
                PropertyValue::String(
                    "no vector index has been written for this branch yet".to_string(),
                ),
            );
            return ai_config_result_rows(vec![row]);
        }

        let mut rows = Vec::with_capacity(partitions.len());
        for partition in partitions {
            let mut row = Row::new();
            row.insert(
                "partition".to_string(),
                PropertyValue::String(partition.to_string()),
            );
            // Which of these the SQL query path actually reads. With more than
            // one partition present, "queried the wrong partition" is a new
            // cause of zero results and this column is what distinguishes it.
            row.insert(
                "queried".to_string(),
                PropertyValue::Boolean(configured.as_ref() == Some(&partition)),
            );

            match engine.stats(&self.tenant_id, &self.repo_id, &branch, &partition) {
                Ok(stats) => {
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
                    row.insert(
                        "quantization".to_string(),
                        PropertyValue::String(stats.quantization.to_string()),
                    );
                    row.insert(
                        "metric".to_string(),
                        PropertyValue::String(stats.distance_metric.to_string()),
                    );
                }
                Err(e) => {
                    row.insert(
                        "status".to_string(),
                        PropertyValue::String("error".to_string()),
                    );
                    row.insert(
                        "details".to_string(),
                        PropertyValue::String(format!("{}", e)),
                    );
                }
            }
            rows.push(row);
        }

        ai_config_result_rows(rows)
    }

    async fn execute_verify_vector_index(&self) -> Result<RowStream, Error> {
        let engine = self
            .hnsw_engine
            .as_ref()
            .ok_or_else(|| Error::Validation("HNSW engine not configured".to_string()))?;

        let branch = self.effective_branch().await;

        // Get HNSW index count, summed over every PARTITION on the branch.
        //
        // Per-partition, because `list_embeddings` below counts every row in
        // `cf::EMBEDDINGS` regardless of which embedder wrote it. Comparing a
        // branch-wide row count against ONE partition's vector count would
        // report a permanent mismatch the moment a second embedding space
        // existed — the same shape of false alarm that the workspace fix
        // removed from the other side of this comparison.
        let partitions = engine
            .list_partitions(&self.tenant_id, &self.repo_id, &branch)
            .unwrap_or_default();
        let hnsw_count: usize = partitions
            .iter()
            .filter_map(|p| {
                engine
                    .stats(&self.tenant_id, &self.repo_id, &branch, p)
                    .ok()
                    .map(|s| s.count)
            })
            .sum();

        // Get embedding storage count.
        //
        // `engine.stats` above counts the whole branch, across every workspace.
        // This side used to count only the workspace literally named "default",
        // so any deployment with content elsewhere compared a branch-wide
        // number against a one-workspace number and reported a permanent
        // "mismatch" that no REBUILD could ever clear. Sum the same set the
        // engine covers.
        //
        // And count INDEX ENTRIES, not nodes. `list_embeddings` returns one row
        // per source, so a chunked corpus compared a per-node count against the
        // index's per-chunk count: a healthy 31-vector index over 9 documents
        // reported `mismatch 31/9` and told the operator to run a REBUILD — the
        // one command that would then actually break it. `list_index_entries`
        // is the unit the index stores, and it is the same list the rebuild
        // iterates, so agreement here means the two really do agree.
        let storage_count = if let Some(ref emb_storage) = self.embedding_storage {
            match emb_storage.list_workspaces(&self.tenant_id, &self.repo_id, &branch) {
                Ok(workspaces) => workspaces
                    .iter()
                    .map(|ws| {
                        emb_storage
                            .list_index_entries(&self.tenant_id, &self.repo_id, &branch, ws)
                            .map(|list| list.len())
                            .unwrap_or(0)
                    })
                    .sum(),
                Err(_) => 0,
            }
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
}

/// One `SHOW AI PROVIDERS` row.
fn provider_row(p: &AIProviderConfig) -> Row {
    let mut row = Row::new();
    row.insert("slug".to_string(), PropertyValue::String(p.slug.clone()));
    row.insert(
        "kind".to_string(),
        PropertyValue::String(p.kind.serde_name().to_string()),
    );
    row.insert("enabled".to_string(), PropertyValue::Boolean(p.enabled));
    row.insert(
        "has_api_key".to_string(),
        PropertyValue::Boolean(p.api_key_encrypted.is_some()),
    );
    row.insert(
        "api_endpoint".to_string(),
        PropertyValue::String(p.api_endpoint.clone().unwrap_or_default()),
    );
    let models: Vec<String> = p
        .models
        .iter()
        .map(|m| {
            let cases: Vec<&str> = m.use_cases.iter().map(use_case_name).collect();
            let default = if m.is_default { "*" } else { "" };
            format!("{}{}[{}]", m.model_id, default, cases.join(","))
        })
        .collect();
    row.insert(
        "models".to_string(),
        PropertyValue::String(models.join(" ")),
    );
    row
}

fn use_case_name(c: &AIUseCase) -> &'static str {
    match c {
        AIUseCase::Embedding => "embedding",
        AIUseCase::Chat => "chat",
        AIUseCase::Agent => "agent",
        AIUseCase::Completion => "completion",
        AIUseCase::Classification => "classification",
    }
}

fn parse_use_case(raw: &str) -> Result<AIUseCase, Error> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "embedding" | "embeddings" => Ok(AIUseCase::Embedding),
        "chat" => Ok(AIUseCase::Chat),
        "agent" => Ok(AIUseCase::Agent),
        "completion" => Ok(AIUseCase::Completion),
        "classification" => Ok(AIUseCase::Classification),
        other => Err(Error::Validation(format!(
            "Unknown use case '{}'. Supported: embedding, chat, agent, completion, classification",
            other
        ))),
    }
}

/// Apply `ADD PROVIDER` / `DROP PROVIDER` to a tenant's provider list.
///
/// `ADD PROVIDER '<slug>'` creates the entry or updates it in place. SET keys:
/// `KIND` (a provider kind; defaults to the slug when the slug is itself a
/// kind name, required otherwise), `API_KEY` (encrypted with `encrypt`),
/// `BASE_URL` / `ENDPOINT`, `DISPLAY_NAME`, `ENABLED`, `MODEL` (repeatable, or
/// a comma-separated list; the first becomes the default), `USE_CASES`
/// (comma-separated, applied to the models named in the same statement;
/// default `chat,agent`). A field that is not SET keeps its stored value, so
/// re-running the statement without `API_KEY` leaves the key in place.
///
/// `DROP PROVIDER '<slug>'` removes the entry; an unknown slug is an error.
///
/// Pure so it can be tested without a store or a master key.
fn apply_provider_operation(
    config: &mut TenantAIConfig,
    operation: &AIConfigOperation,
    encrypt: impl Fn(&str) -> Result<Vec<u8>, Error>,
) -> Result<String, Error> {
    match operation {
        AIConfigOperation::AddProvider { provider, settings } => {
            let slug = provider.trim().to_ascii_lowercase();
            if slug.is_empty() {
                return Err(Error::Validation("Provider slug must not be empty".into()));
            }

            let mut kind: Option<AIProviderKind> = None;
            let mut api_key: Option<Vec<u8>> = None;
            let mut endpoint: Option<Option<String>> = None;
            let mut display_name: Option<String> = None;
            let mut enabled: Option<bool> = None;
            let mut model_ids: Vec<String> = Vec::new();
            let mut use_cases: Option<Vec<AIUseCase>> = None;

            for setting in settings {
                let value = setting.value.trim();
                match setting.key.to_uppercase().as_str() {
                    "KIND" | "PROVIDER" => {
                        kind = Some(
                            AIProviderKind::from_serde_name(&value.to_ascii_lowercase())
                                .ok_or_else(|| {
                                    Error::Validation(format!(
                                    "Unknown provider kind '{}'. Supported: openai, anthropic, \
                                     google, ollama, azure_openai, groq, openrouter, bedrock, \
                                     custom, local",
                                    value
                                ))
                                })?,
                        );
                    }
                    "API_KEY" => api_key = Some(encrypt(value)?),
                    "BASE_URL" | "ENDPOINT" | "API_ENDPOINT" => {
                        endpoint = Some(if value.is_empty() {
                            None
                        } else {
                            Some(value.to_string())
                        });
                    }
                    "DISPLAY_NAME" => display_name = Some(value.to_string()),
                    "ENABLED" => {
                        enabled = Some(parse_bool(value).map_err(|_| {
                            Error::Validation(format!(
                                "Invalid boolean value for ENABLED: {}",
                                value
                            ))
                        })?)
                    }
                    "MODEL" | "MODELS" => {
                        model_ids.extend(
                            value
                                .split(',')
                                .map(str::trim)
                                .filter(|m| !m.is_empty())
                                .map(String::from),
                        );
                    }
                    "USE_CASES" | "USE_CASE" => {
                        let parsed = value
                            .split(',')
                            .filter(|c| !c.trim().is_empty())
                            .map(parse_use_case)
                            .collect::<Result<Vec<_>, _>>()?;
                        if parsed.is_empty() {
                            return Err(Error::Validation(
                                "USE_CASES must name at least one use case".into(),
                            ));
                        }
                        use_cases = Some(parsed);
                    }
                    other => {
                        return Err(Error::Validation(format!(
                            "Unknown provider setting '{}'. Supported: KIND, API_KEY, BASE_URL, \
                             DISPLAY_NAME, ENABLED, MODEL, USE_CASES",
                            other
                        )));
                    }
                }
            }

            let existing = config.providers.iter_mut().find(|p| p.slug == slug);

            let entry: &mut AIProviderConfig = match existing {
                Some(entry) => {
                    if let Some(k) = kind {
                        if k != entry.kind {
                            return Err(Error::Validation(format!(
                                "Provider '{}' already exists with kind '{}'; a slug's kind cannot \
                                 be changed. Create a new slug instead.",
                                slug,
                                entry.kind.serde_name()
                            )));
                        }
                    }
                    entry
                }
                None => {
                    let kind = kind
                        .or_else(|| AIProviderKind::from_serde_name(&slug))
                        .ok_or_else(|| {
                            Error::Validation(format!(
                                "Provider '{}' does not exist yet; add SET KIND = '<openai|anthropic|\
                                 google|ollama|azure_openai|groq|openrouter|bedrock|custom|local>' \
                                 to create it.",
                                slug
                            ))
                        })?;
                    config.providers.push(AIProviderConfig {
                        slug: slug.clone(),
                        kind,
                        display_name: None,
                        icon_url: None,
                        api_key_encrypted: None,
                        api_endpoint: None,
                        enabled: true,
                        models: Vec::new(),
                    });
                    config
                        .providers
                        .last_mut()
                        .expect("provider was just pushed")
                }
            };

            if let Some(key) = api_key {
                entry.api_key_encrypted = Some(key);
            }
            if let Some(endpoint) = endpoint {
                entry.api_endpoint = endpoint;
            }
            if let Some(name) = display_name {
                entry.display_name = Some(name);
            }
            if let Some(enabled) = enabled {
                entry.enabled = enabled;
            }
            if !model_ids.is_empty() {
                let cases = use_cases.unwrap_or_else(|| vec![AIUseCase::Chat, AIUseCase::Agent]);
                let is_embedding = cases.contains(&AIUseCase::Embedding);
                // A model added again is replaced rather than duplicated.
                entry.models.retain(|m| !model_ids.contains(&m.model_id));
                let has_default_for = |models: &[AIModelConfig], case: &AIUseCase| {
                    models
                        .iter()
                        .any(|m| m.is_default && m.use_cases.contains(case))
                };
                for model_id in model_ids {
                    let is_default = cases.iter().any(|c| !has_default_for(&entry.models, c));
                    entry.models.push(AIModelConfig {
                        model_id,
                        display_name: String::new(),
                        use_cases: cases.clone(),
                        default_temperature: if is_embedding { 0.0 } else { 0.7 },
                        default_max_tokens: if is_embedding { 0 } else { 4096 },
                        is_default,
                        metadata: None,
                    });
                    let last = entry.models.last_mut().expect("model was just pushed");
                    last.display_name = last.model_id.clone();
                }
            } else if use_cases.is_some() {
                return Err(Error::Validation(
                    "USE_CASES applies to the models named in the same statement; add SET MODEL = '...'".into(),
                ));
            }

            Ok(format!(
                "Provider '{}' ({}) configured with {} model(s)",
                entry.slug,
                entry.kind.serde_name(),
                entry.models.len()
            ))
        }
        AIConfigOperation::DropProvider { provider } => {
            let slug = provider.trim().to_ascii_lowercase();
            let before = config.providers.len();
            config.providers.retain(|p| p.slug != slug);
            if config.providers.len() == before {
                let known: Vec<&str> = config.providers.iter().map(|p| p.slug.as_str()).collect();
                return Err(Error::Validation(format!(
                    "Provider '{}' is not configured (configured: {})",
                    slug,
                    if known.is_empty() {
                        "none".to_string()
                    } else {
                        known.join(", ")
                    }
                )));
            }
            Ok(format!("Provider '{}' removed", slug))
        }
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

#[cfg(test)]
mod provider_operation_tests {
    use super::*;

    fn setting(key: &str, value: &str) -> ConfigSetting {
        ConfigSetting {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn encrypt(plain: &str) -> Result<Vec<u8>, Error> {
        Ok(format!("enc:{plain}").into_bytes())
    }

    fn add(slug: &str, settings: Vec<ConfigSetting>) -> AIConfigOperation {
        AIConfigOperation::AddProvider {
            provider: slug.to_string(),
            settings,
        }
    }

    #[test]
    fn add_provider_creates_an_entry_in_the_provider_list() {
        let mut config = TenantAIConfig::new("t".to_string());
        let msg = apply_provider_operation(
            &mut config,
            &add(
                "bedrock",
                vec![
                    setting("API_KEY", "AKIA:secret"),
                    setting("BASE_URL", "us-east-1"),
                    setting("MODEL", "anthropic.claude-sonnet-4-20250514-v1:0"),
                ],
            ),
            encrypt,
        )
        .unwrap();
        assert!(msg.contains("bedrock"));
        assert_eq!(config.providers.len(), 1);
        let p = &config.providers[0];
        assert_eq!(p.slug, "bedrock");
        assert_eq!(p.kind, AIProviderKind::Bedrock);
        assert_eq!(
            p.api_key_encrypted.as_deref(),
            Some(b"enc:AKIA:secret".as_slice())
        );
        assert_eq!(p.api_endpoint.as_deref(), Some("us-east-1"));
        assert!(p.enabled);
        assert_eq!(p.models.len(), 1);
        assert!(p.models[0].is_default);
        assert_eq!(
            p.models[0].use_cases,
            vec![AIUseCase::Chat, AIUseCase::Agent]
        );
    }

    #[test]
    fn a_slug_that_is_not_a_kind_needs_kind() {
        let mut config = TenantAIConfig::new("t".to_string());
        let err = apply_provider_operation(&mut config, &add("gateway", vec![]), encrypt)
            .unwrap_err()
            .to_string();
        assert!(err.contains("SET KIND"), "{err}");

        apply_provider_operation(
            &mut config,
            &add(
                "gateway",
                vec![setting("KIND", "custom"), setting("ENDPOINT", "http://gw")],
            ),
            encrypt,
        )
        .unwrap();
        assert_eq!(config.providers[0].kind, AIProviderKind::Custom);
        assert_eq!(
            config.providers[0].api_endpoint.as_deref(),
            Some("http://gw")
        );
    }

    #[test]
    fn update_keeps_the_stored_key_and_models_and_refuses_a_kind_change() {
        let mut config = TenantAIConfig::new("t".to_string());
        apply_provider_operation(
            &mut config,
            &add(
                "openai",
                vec![setting("API_KEY", "sk-1"), setting("MODEL", "gpt-4o")],
            ),
            encrypt,
        )
        .unwrap();
        apply_provider_operation(
            &mut config,
            &add("openai", vec![setting("ENABLED", "false")]),
            encrypt,
        )
        .unwrap();
        let p = &config.providers[0];
        assert_eq!(p.api_key_encrypted.as_deref(), Some(b"enc:sk-1".as_slice()));
        assert_eq!(p.models.len(), 1);
        assert!(!p.enabled);

        let err = apply_provider_operation(
            &mut config,
            &add("openai", vec![setting("KIND", "groq")]),
            encrypt,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("cannot be changed"), "{err}");
    }

    #[test]
    fn embedding_models_get_their_own_default() {
        let mut config = TenantAIConfig::new("t".to_string());
        apply_provider_operation(
            &mut config,
            &add("openai", vec![setting("MODEL", "gpt-4o, gpt-4o-mini")]),
            encrypt,
        )
        .unwrap();
        apply_provider_operation(
            &mut config,
            &add(
                "openai",
                vec![
                    setting("MODEL", "text-embedding-3-small"),
                    setting("USE_CASES", "embedding"),
                ],
            ),
            encrypt,
        )
        .unwrap();
        let models = &config.providers[0].models;
        assert_eq!(models.len(), 3);
        assert!(models[0].is_default && models[0].model_id == "gpt-4o");
        assert!(!models[1].is_default);
        let emb = &models[2];
        assert_eq!(emb.use_cases, vec![AIUseCase::Embedding]);
        assert!(
            emb.is_default,
            "first embedding model is the embedding default"
        );
        assert_eq!(emb.default_max_tokens, 0);
        assert_eq!(
            config
                .get_default_model(AIUseCase::Embedding)
                .map(|m| m.model_id.as_str()),
            Some("text-embedding-3-small")
        );
        assert_eq!(
            config
                .get_default_model(AIUseCase::Chat)
                .map(|m| m.model_id.as_str()),
            Some("gpt-4o")
        );
    }

    #[test]
    fn drop_provider_removes_the_entry_and_names_the_rest_on_a_miss() {
        let mut config = TenantAIConfig::new("t".to_string());
        apply_provider_operation(&mut config, &add("openai", vec![]), encrypt).unwrap();
        apply_provider_operation(&mut config, &add("ollama", vec![]), encrypt).unwrap();

        let drop = AIConfigOperation::DropProvider {
            provider: "openai".to_string(),
        };
        apply_provider_operation(&mut config, &drop, encrypt).unwrap();
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.providers[0].slug, "ollama");

        let err = apply_provider_operation(&mut config, &drop, encrypt)
            .unwrap_err()
            .to_string();
        assert!(err.contains("ollama"), "{err}");
    }

    #[test]
    fn unknown_setting_and_use_case_are_errors() {
        let mut config = TenantAIConfig::new("t".to_string());
        let err = apply_provider_operation(
            &mut config,
            &add("openai", vec![setting("DIMENSIONS", "1536")]),
            encrypt,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Unknown provider setting"), "{err}");

        let err = apply_provider_operation(
            &mut config,
            &add(
                "openai",
                vec![setting("MODEL", "x"), setting("USE_CASES", "vision")],
            ),
            encrypt,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("Unknown use case"), "{err}");
    }
}
