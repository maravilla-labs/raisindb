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
