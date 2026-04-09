use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSetting {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AIConfigOperation {
    AddProvider {
        provider: String,
        settings: Vec<ConfigSetting>,
    },
    DropProvider {
        provider: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AIConfigStatement {
    // Embedding config
    AlterEmbeddingConfig { settings: Vec<ConfigSetting> },
    ShowEmbeddingConfig,
    TestEmbeddingConnection,
    // AI provider config
    AlterAIConfig { operation: AIConfigOperation },
    ShowAIProviders,
    ShowAIConfig,
    TestAIProvider { provider: String },
    // Vector index management
    RebuildVectorIndex,
    RegenerateEmbeddings,
    ShowVectorIndexHealth,
    VerifyVectorIndex,
}

impl AIConfigStatement {
    pub fn operation(&self) -> &'static str {
        match self {
            AIConfigStatement::AlterEmbeddingConfig { .. } => "ALTER EMBEDDING CONFIG",
            AIConfigStatement::ShowEmbeddingConfig => "SHOW EMBEDDING CONFIG",
            AIConfigStatement::TestEmbeddingConnection => "TEST EMBEDDING CONNECTION",
            AIConfigStatement::AlterAIConfig { operation } => match operation {
                AIConfigOperation::AddProvider { .. } => "ALTER AI CONFIG ADD PROVIDER",
                AIConfigOperation::DropProvider { .. } => "ALTER AI CONFIG DROP PROVIDER",
            },
            AIConfigStatement::ShowAIProviders => "SHOW AI PROVIDERS",
            AIConfigStatement::ShowAIConfig => "SHOW AI CONFIG",
            AIConfigStatement::TestAIProvider { .. } => "TEST AI PROVIDER",
            AIConfigStatement::RebuildVectorIndex => "REBUILD VECTOR INDEX",
            AIConfigStatement::RegenerateEmbeddings => "REGENERATE EMBEDDINGS",
            AIConfigStatement::ShowVectorIndexHealth => "SHOW VECTOR INDEX HEALTH",
            AIConfigStatement::VerifyVectorIndex => "VERIFY VECTOR INDEX",
        }
    }

    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            AIConfigStatement::ShowEmbeddingConfig
                | AIConfigStatement::TestEmbeddingConnection
                | AIConfigStatement::ShowAIProviders
                | AIConfigStatement::ShowAIConfig
                | AIConfigStatement::TestAIProvider { .. }
                | AIConfigStatement::ShowVectorIndexHealth
        )
    }
}

impl fmt::Display for AIConfigStatement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AIConfigStatement::AlterEmbeddingConfig { settings } => {
                write!(f, "ALTER EMBEDDING CONFIG")?;
                for s in settings {
                    write!(f, " SET {} = '{}'", s.key, s.value)?;
                }
                Ok(())
            }
            AIConfigStatement::ShowEmbeddingConfig => write!(f, "SHOW EMBEDDING CONFIG"),
            AIConfigStatement::TestEmbeddingConnection => write!(f, "TEST EMBEDDING CONNECTION"),
            AIConfigStatement::AlterAIConfig { operation } => match operation {
                AIConfigOperation::AddProvider { provider, settings } => {
                    write!(f, "ALTER AI CONFIG ADD PROVIDER '{}'", provider)?;
                    for s in settings {
                        write!(f, " SET {} = '{}'", s.key, s.value)?;
                    }
                    Ok(())
                }
                AIConfigOperation::DropProvider { provider } => {
                    write!(f, "ALTER AI CONFIG DROP PROVIDER '{}'", provider)
                }
            },
            AIConfigStatement::ShowAIProviders => write!(f, "SHOW AI PROVIDERS"),
            AIConfigStatement::ShowAIConfig => write!(f, "SHOW AI CONFIG"),
            AIConfigStatement::TestAIProvider { provider } => {
                write!(f, "TEST AI PROVIDER '{}'", provider)
            }
            AIConfigStatement::RebuildVectorIndex => write!(f, "REBUILD VECTOR INDEX"),
            AIConfigStatement::RegenerateEmbeddings => write!(f, "REGENERATE EMBEDDINGS"),
            AIConfigStatement::ShowVectorIndexHealth => write!(f, "SHOW VECTOR INDEX HEALTH"),
            AIConfigStatement::VerifyVectorIndex => write!(f, "VERIFY VECTOR INDEX"),
        }
    }
}
