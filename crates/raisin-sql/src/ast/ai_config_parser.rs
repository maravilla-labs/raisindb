use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_until, take_while1},
    character::complete::{char, multispace0, multispace1},
    combinator::map,
    IResult, Parser,
};

use super::ai_config::{AIConfigOperation, AIConfigStatement, ConfigSetting};

#[derive(Debug, Clone, PartialEq)]
pub struct AIConfigParseError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for AIConfigParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(pos) = self.position {
            write!(
                f,
                "AI config parse error at position {}: {}",
                pos, self.message
            )
        } else {
            write!(f, "AI config parse error: {}", self.message)
        }
    }
}

impl std::error::Error for AIConfigParseError {}

// ---------------------------------------------------------------------------
// Guard function
// ---------------------------------------------------------------------------

pub fn is_ai_config_statement(sql: &str) -> bool {
    let upper = sql.trim().to_uppercase();

    upper.starts_with("ALTER EMBEDDING CONFIG")
        || upper.starts_with("SHOW EMBEDDING CONFIG")
        || upper.starts_with("TEST EMBEDDING")
        || upper.starts_with("ALTER AI CONFIG")
        || upper.starts_with("SHOW AI PROVIDERS")
        || upper.starts_with("SHOW AI CONFIG")
        || upper.starts_with("TEST AI PROVIDER")
        || upper.starts_with("REBUILD VECTOR INDEX")
        || upper.starts_with("REGENERATE EMBEDDINGS")
        || upper.starts_with("SHOW VECTOR INDEX")
        || upper.starts_with("VERIFY VECTOR INDEX")
}

// ---------------------------------------------------------------------------
// Public parser entry point
// ---------------------------------------------------------------------------

pub fn parse_ai_config(sql: &str) -> Result<Option<AIConfigStatement>, AIConfigParseError> {
    let trimmed = sql.trim();

    // Strip leading SQL comments
    let statement_start = super::ddl_parser::strip_leading_comments(trimmed);

    if !is_ai_config_statement(statement_start) {
        return Ok(None);
    }

    let offset_to_statement_start = statement_start.as_ptr() as usize - sql.as_ptr() as usize;

    match ai_config_statement(statement_start) {
        Ok((remaining, stmt)) => {
            let remaining_trimmed = remaining.trim().trim_end_matches(';').trim();
            if remaining_trimmed.is_empty() {
                Ok(Some(stmt))
            } else {
                let position_in_statement = statement_start.len() - remaining.len();
                let position = offset_to_statement_start + position_in_statement;
                Err(AIConfigParseError {
                    message: format!("Unexpected trailing content: '{}'", remaining_trimmed),
                    position: Some(position),
                })
            }
        }
        Err(e) => {
            let (position, message) = match &e {
                nom::Err::Failure(err) | nom::Err::Error(err) => {
                    let pos_in_statement = statement_start.len() - err.input.len();
                    let remaining = err.input.trim();
                    let problematic: String = remaining
                        .chars()
                        .take(30)
                        .take_while(|c| *c != '\n')
                        .collect();
                    (
                        Some(offset_to_statement_start + pos_in_statement),
                        format!("Parse error near: '{}'", problematic.trim()),
                    )
                }
                nom::Err::Incomplete(_) => (None, "Incomplete AI config statement".to_string()),
            };
            Err(AIConfigParseError { message, position })
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level statement dispatcher
// ---------------------------------------------------------------------------

fn ai_config_statement(input: &str) -> IResult<&str, AIConfigStatement> {
    alt((
        alter_embedding_config,
        show_embedding_config,
        test_embedding_connection,
        alter_ai_config,
        show_ai_providers,
        show_ai_config,
        test_ai_provider,
        rebuild_vector_index,
        regenerate_embeddings,
        show_vector_index_health,
        verify_vector_index,
    ))
    .parse(input)
}

// ---------------------------------------------------------------------------
// Primitive parsers
// ---------------------------------------------------------------------------

fn quoted_string(input: &str) -> IResult<&str, &str> {
    alt((
        nom::sequence::delimited(char('\''), take_until("'"), char('\'')),
        nom::sequence::delimited(char('"'), take_until("\""), char('"')),
    ))
    .parse(input)
}

fn unquoted_value(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| !c.is_whitespace() && c != ';').parse(input)
}

fn config_value(input: &str) -> IResult<&str, String> {
    alt((
        map(quoted_string, |s: &str| s.to_string()),
        map(unquoted_value, |s: &str| s.to_string()),
    ))
    .parse(input)
}

fn set_clause(input: &str) -> IResult<&str, ConfigSetting> {
    let (input, _) = tag_no_case("SET").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, key) = map(
        take_while1(|c: char| c.is_alphanumeric() || c == '_'),
        |s: &str| s.to_uppercase(),
    )
    .parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('=').parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, value) = config_value(input)?;
    Ok((input, ConfigSetting { key, value }))
}

fn set_clauses(input: &str) -> IResult<&str, Vec<ConfigSetting>> {
    let mut settings = Vec::new();
    let mut remaining = input;
    loop {
        let trimmed = remaining.trim_start();
        if let Ok((rest, setting)) = set_clause(trimmed) {
            settings.push(setting);
            remaining = rest;
        } else {
            break;
        }
    }
    if settings.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }
    Ok((remaining, settings))
}

// ---------------------------------------------------------------------------
// Individual statement parsers
// ---------------------------------------------------------------------------

fn alter_embedding_config(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("ALTER").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("EMBEDDING").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("CONFIG").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, settings) = set_clauses(input)?;
    Ok((input, AIConfigStatement::AlterEmbeddingConfig { settings }))
}

fn show_embedding_config(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("EMBEDDING").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("CONFIG").parse(input)?;
    Ok((input, AIConfigStatement::ShowEmbeddingConfig))
}

fn test_embedding_connection(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("TEST").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("EMBEDDING").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("CONNECTION").parse(input)?;
    Ok((input, AIConfigStatement::TestEmbeddingConnection))
}

fn alter_ai_config(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("ALTER").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("AI").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("CONFIG").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, operation) = alt((add_provider, drop_provider)).parse(input)?;
    Ok((input, AIConfigStatement::AlterAIConfig { operation }))
}

fn add_provider(input: &str) -> IResult<&str, AIConfigOperation> {
    let (input, _) = tag_no_case("ADD").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("PROVIDER").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, provider) = map(quoted_string, |s: &str| s.to_string()).parse(input)?;
    // Optional SET clauses
    let mut settings = Vec::new();
    let mut remaining = input;
    loop {
        let trimmed = remaining.trim_start();
        if let Ok((rest, setting)) = set_clause(trimmed) {
            settings.push(setting);
            remaining = rest;
        } else {
            break;
        }
    }
    Ok((
        remaining,
        AIConfigOperation::AddProvider { provider, settings },
    ))
}

fn drop_provider(input: &str) -> IResult<&str, AIConfigOperation> {
    let (input, _) = tag_no_case("DROP").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("PROVIDER").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, provider) = map(quoted_string, |s: &str| s.to_string()).parse(input)?;
    Ok((input, AIConfigOperation::DropProvider { provider }))
}

fn show_ai_providers(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("AI").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("PROVIDERS").parse(input)?;
    Ok((input, AIConfigStatement::ShowAIProviders))
}

fn show_ai_config(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("AI").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("CONFIG").parse(input)?;
    Ok((input, AIConfigStatement::ShowAIConfig))
}

fn test_ai_provider(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("TEST").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("AI").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("PROVIDER").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, provider) = map(quoted_string, |s: &str| s.to_string()).parse(input)?;
    Ok((input, AIConfigStatement::TestAIProvider { provider }))
}

fn rebuild_vector_index(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("REBUILD").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("VECTOR").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("INDEX").parse(input)?;
    Ok((input, AIConfigStatement::RebuildVectorIndex))
}

fn regenerate_embeddings(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("REGENERATE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("EMBEDDINGS").parse(input)?;
    Ok((input, AIConfigStatement::RegenerateEmbeddings))
}

fn show_vector_index_health(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("VECTOR").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("INDEX").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("HEALTH").parse(input)?;
    Ok((input, AIConfigStatement::ShowVectorIndexHealth))
}

fn verify_vector_index(input: &str) -> IResult<&str, AIConfigStatement> {
    let (input, _) = tag_no_case("VERIFY").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("VECTOR").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("INDEX").parse(input)?;
    Ok((input, AIConfigStatement::VerifyVectorIndex))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_show_embedding_config() {
        let result = parse_ai_config("SHOW EMBEDDING CONFIG").unwrap().unwrap();
        assert_eq!(result, AIConfigStatement::ShowEmbeddingConfig);
    }

    #[test]
    fn test_parse_alter_embedding_config() {
        let result = parse_ai_config("ALTER EMBEDDING CONFIG SET PROVIDER = 'openai'")
            .unwrap()
            .unwrap();
        match result {
            AIConfigStatement::AlterEmbeddingConfig { settings } => {
                assert_eq!(settings.len(), 1);
                assert_eq!(settings[0].key, "PROVIDER");
                assert_eq!(settings[0].value, "openai");
            }
            other => panic!("Expected AlterEmbeddingConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_alter_embedding_config_multiple_settings() {
        let sql =
            "ALTER EMBEDDING CONFIG SET PROVIDER = 'openai' SET MODEL = 'text-embedding-3-small' SET API_KEY = 'sk-test123' SET ENABLED = true";
        let result = parse_ai_config(sql).unwrap().unwrap();
        match result {
            AIConfigStatement::AlterEmbeddingConfig { settings } => {
                assert_eq!(settings.len(), 4);
                assert_eq!(settings[0].key, "PROVIDER");
                assert_eq!(settings[0].value, "openai");
                assert_eq!(settings[1].key, "MODEL");
                assert_eq!(settings[1].value, "text-embedding-3-small");
                assert_eq!(settings[2].key, "API_KEY");
                assert_eq!(settings[2].value, "sk-test123");
                assert_eq!(settings[3].key, "ENABLED");
                assert_eq!(settings[3].value, "true");
            }
            other => panic!("Expected AlterEmbeddingConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_test_embedding_connection() {
        let result = parse_ai_config("TEST EMBEDDING CONNECTION")
            .unwrap()
            .unwrap();
        assert_eq!(result, AIConfigStatement::TestEmbeddingConnection);
    }

    #[test]
    fn test_parse_show_ai_providers() {
        let result = parse_ai_config("SHOW AI PROVIDERS").unwrap().unwrap();
        assert_eq!(result, AIConfigStatement::ShowAIProviders);
    }

    #[test]
    fn test_parse_show_ai_config() {
        let result = parse_ai_config("SHOW AI CONFIG").unwrap().unwrap();
        assert_eq!(result, AIConfigStatement::ShowAIConfig);
    }

    #[test]
    fn test_parse_alter_ai_config_add_provider() {
        let sql = "ALTER AI CONFIG ADD PROVIDER 'anthropic' SET API_KEY = 'sk-ant-test' SET ENDPOINT = 'https://api.anthropic.com' SET ENABLED = true";
        let result = parse_ai_config(sql).unwrap().unwrap();
        match result {
            AIConfigStatement::AlterAIConfig { operation } => match operation {
                AIConfigOperation::AddProvider { provider, settings } => {
                    assert_eq!(provider, "anthropic");
                    assert_eq!(settings.len(), 3);
                    assert_eq!(settings[0].key, "API_KEY");
                    assert_eq!(settings[0].value, "sk-ant-test");
                    assert_eq!(settings[1].key, "ENDPOINT");
                    assert_eq!(settings[1].value, "https://api.anthropic.com");
                    assert_eq!(settings[2].key, "ENABLED");
                    assert_eq!(settings[2].value, "true");
                }
                other => panic!("Expected AddProvider, got {:?}", other),
            },
            other => panic!("Expected AlterAIConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_alter_ai_config_drop_provider() {
        let result = parse_ai_config("ALTER AI CONFIG DROP PROVIDER 'anthropic'")
            .unwrap()
            .unwrap();
        match result {
            AIConfigStatement::AlterAIConfig { operation } => match operation {
                AIConfigOperation::DropProvider { provider } => {
                    assert_eq!(provider, "anthropic");
                }
                other => panic!("Expected DropProvider, got {:?}", other),
            },
            other => panic!("Expected AlterAIConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_test_ai_provider() {
        let result = parse_ai_config("TEST AI PROVIDER 'openai'")
            .unwrap()
            .unwrap();
        match result {
            AIConfigStatement::TestAIProvider { provider } => {
                assert_eq!(provider, "openai");
            }
            other => panic!("Expected TestAIProvider, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_rebuild_vector_index() {
        let result = parse_ai_config("REBUILD VECTOR INDEX").unwrap().unwrap();
        assert_eq!(result, AIConfigStatement::RebuildVectorIndex);
    }

    #[test]
    fn test_parse_regenerate_embeddings() {
        let result = parse_ai_config("REGENERATE EMBEDDINGS").unwrap().unwrap();
        assert_eq!(result, AIConfigStatement::RegenerateEmbeddings);
    }

    #[test]
    fn test_parse_show_vector_index_health() {
        let result = parse_ai_config("SHOW VECTOR INDEX HEALTH")
            .unwrap()
            .unwrap();
        assert_eq!(result, AIConfigStatement::ShowVectorIndexHealth);
    }

    #[test]
    fn test_parse_verify_vector_index() {
        let result = parse_ai_config("VERIFY VECTOR INDEX").unwrap().unwrap();
        assert_eq!(result, AIConfigStatement::VerifyVectorIndex);
    }

    #[test]
    fn test_is_ai_config_statement() {
        // Positive cases
        assert!(is_ai_config_statement(
            "ALTER EMBEDDING CONFIG SET PROVIDER = 'openai'"
        ));
        assert!(is_ai_config_statement("SHOW EMBEDDING CONFIG"));
        assert!(is_ai_config_statement("TEST EMBEDDING CONNECTION"));
        assert!(is_ai_config_statement("ALTER AI CONFIG ADD PROVIDER 'x'"));
        assert!(is_ai_config_statement("ALTER AI CONFIG DROP PROVIDER 'x'"));
        assert!(is_ai_config_statement("SHOW AI PROVIDERS"));
        assert!(is_ai_config_statement("SHOW AI CONFIG"));
        assert!(is_ai_config_statement("TEST AI PROVIDER 'openai'"));
        assert!(is_ai_config_statement("REBUILD VECTOR INDEX"));
        assert!(is_ai_config_statement("REGENERATE EMBEDDINGS"));
        assert!(is_ai_config_statement("SHOW VECTOR INDEX HEALTH"));
        assert!(is_ai_config_statement("VERIFY VECTOR INDEX"));

        // Case insensitivity
        assert!(is_ai_config_statement("show embedding config"));
        assert!(is_ai_config_statement("Show Ai Providers"));

        // With leading whitespace
        assert!(is_ai_config_statement("  SHOW EMBEDDING CONFIG"));

        // Negative cases
        assert!(!is_ai_config_statement("SELECT * FROM nodes"));
        assert!(!is_ai_config_statement("CREATE BRANCH 'main'"));
        assert!(!is_ai_config_statement("SHOW BRANCHES"));
        assert!(!is_ai_config_statement("ALTER ROLE 'admin'"));
        assert!(!is_ai_config_statement("INSERT INTO nodes VALUES (1)"));
    }

    #[test]
    fn test_trailing_semicolon() {
        let result = parse_ai_config("SHOW EMBEDDING CONFIG;").unwrap().unwrap();
        assert_eq!(result, AIConfigStatement::ShowEmbeddingConfig);
    }

    #[test]
    fn test_add_provider_no_settings() {
        let result = parse_ai_config("ALTER AI CONFIG ADD PROVIDER 'minimal'")
            .unwrap()
            .unwrap();
        match result {
            AIConfigStatement::AlterAIConfig { operation } => match operation {
                AIConfigOperation::AddProvider { provider, settings } => {
                    assert_eq!(provider, "minimal");
                    assert!(settings.is_empty());
                }
                other => panic!("Expected AddProvider, got {:?}", other),
            },
            other => panic!("Expected AlterAIConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_not_ai_config_returns_none() {
        let result = parse_ai_config("SELECT 1").unwrap();
        assert!(result.is_none());
    }
}
