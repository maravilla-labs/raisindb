//! SQL context analysis
//!
//! Determines the semantic SQL context at a cursor position by tokenizing
//! and analyzing the text before the cursor. Used to drive context-aware completions.

use super::tokenizer::{tokenize, Token};
use super::types::{AnalyzedContext, SqlContext};

/// Analyze SQL context at cursor position
pub fn analyze_context(sql: &str, cursor_offset: usize) -> AnalyzedContext {
    // Get text before cursor
    let text_before = if cursor_offset <= sql.len() {
        &sql[..cursor_offset]
    } else {
        sql
    };

    // Extract partial word at cursor
    let partial_word = extract_partial_word(text_before);
    let text_for_analysis = &text_before[..text_before.len() - partial_word.len()];

    // Tokenize text before cursor for context detection
    let tokens = tokenize(text_for_analysis);

    // Build context
    let mut ctx = AnalyzedContext {
        partial_word,
        ..Default::default()
    };

    // Extract aliases from the FULL SQL text (not just before cursor)
    // This allows completion of aliases defined after the cursor position
    let full_tokens = tokenize(sql);
    extract_aliases(&full_tokens, &mut ctx);

    // Detect context from tokens before cursor
    ctx.context = detect_context(&tokens);

    // Check for transaction
    ctx.in_transaction = check_in_transaction(&tokens);

    ctx
}

/// Extract partial word at cursor
fn extract_partial_word(text: &str) -> String {
    let mut word = String::new();
    for c in text.chars().rev() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            break;
        }
    }
    word.chars().rev().collect()
}

/// Extract table aliases from tokens
fn extract_aliases(tokens: &[Token], ctx: &mut AnalyzedContext) {
    let mut i = 0;
    while i < tokens.len() {
        // Look for FROM table [AS] alias or JOIN table [AS] alias
        if matches!(
            &tokens[i],
            Token::Keyword(k) if k == "FROM" || k == "JOIN"
        ) {
            i += 1;
            // Skip to table name
            while i < tokens.len()
                && matches!(
                    &tokens[i],
                    Token::Keyword(k) if k == "LEFT" || k == "RIGHT" || k == "INNER" || k == "OUTER" || k == "CROSS" || k == "FULL"
                )
            {
                i += 1;
            }

            // Get table name
            if i < tokens.len() {
                if let Token::Identifier(table) = &tokens[i] {
                    let table_name = table.clone();
                    ctx.from_tables.push(table_name.clone());
                    i += 1;

                    // Check for alias
                    if i < tokens.len() {
                        // Skip AS if present
                        if matches!(&tokens[i], Token::Keyword(k) if k == "AS") {
                            i += 1;
                        }
                        // Get alias
                        if i < tokens.len() {
                            if let Token::Identifier(alias) = &tokens[i] {
                                ctx.aliases.insert(alias.clone(), table_name.clone());
                                i += 1;
                                continue;
                            }
                        }
                    }
                    // No explicit alias, table name is its own alias
                    ctx.aliases.insert(table_name.clone(), table_name);
                }
            }
        }
        i += 1;
    }
}

/// Detect SQL context from tokens
fn detect_context(tokens: &[Token]) -> SqlContext {
    if tokens.is_empty() {
        return SqlContext::StatementStart;
    }

    // Check for dot notation (highest priority)
    if tokens.len() >= 2 {
        if let (Token::Identifier(name), Token::Dot) =
            (&tokens[tokens.len() - 2], &tokens[tokens.len() - 1])
        {
            return SqlContext::AfterDot {
                qualifier: name.clone(),
            };
        }
    }
    if let Some(Token::Dot) = tokens.last() {
        if tokens.len() >= 2 {
            if let Token::Identifier(name) = &tokens[tokens.len() - 2] {
                return SqlContext::AfterDot {
                    qualifier: name.clone(),
                };
            }
        }
    }

    // Check for function argument context
    let mut paren_depth = 0;
    let mut func_name: Option<String> = None;
    let mut arg_count = 0;

    for i in (0..tokens.len()).rev() {
        match &tokens[i] {
            Token::CloseParen => paren_depth += 1,
            Token::OpenParen => {
                if paren_depth == 0 {
                    // We're inside this paren
                    if i > 0 {
                        if let Token::Identifier(name) | Token::Keyword(name) = &tokens[i - 1] {
                            func_name = Some(name.clone());
                        }
                    }
                    break;
                }
                paren_depth -= 1;
            }
            Token::Comma if paren_depth == 0 => {
                arg_count += 1;
            }
            _ => {}
        }
    }

    if let Some(name) = func_name {
        // Check if it's a known function (not a subquery or VALUES)
        let upper = name.to_uppercase();
        if !matches!(upper.as_str(), "SELECT" | "VALUES" | "IN" | "EXISTS") {
            return SqlContext::FunctionArgument {
                function_name: upper,
                arg_index: arg_count,
            };
        }
    }

    // Find last significant keyword
    for i in (0..tokens.len()).rev() {
        if let Token::Keyword(kw) = &tokens[i] {
            match kw.as_str() {
                "SELECT" => {
                    // Check if we've passed FROM
                    for token in tokens.iter().skip(i + 1) {
                        if matches!(token, Token::Keyword(k) if k == "FROM") {
                            // We're past SELECT clause
                            break;
                        }
                    }
                    return SqlContext::SelectClause;
                }
                "FROM" => {
                    // Check what follows
                    if i + 1 < tokens.len() {
                        // Already have table, might be in WHERE or JOIN
                        for token in tokens.iter().skip(i + 1) {
                            if let Token::Keyword(k) = token {
                                match k.as_str() {
                                    "WHERE" => return SqlContext::WhereClause,
                                    "JOIN" => return SqlContext::JoinClause,
                                    "GROUP" => return SqlContext::GroupByClause,
                                    "ORDER" => return SqlContext::OrderByClause,
                                    "HAVING" => return SqlContext::HavingClause,
                                    _ => {}
                                }
                            }
                        }
                    }
                    return SqlContext::FromClause;
                }
                "JOIN" => {
                    // Check if we have table and ON
                    let mut has_table = false;
                    let mut has_on = false;
                    for token in tokens.iter().skip(i + 1) {
                        match token {
                            Token::Identifier(_) if !has_table => has_table = true,
                            Token::Keyword(k) if k == "ON" => has_on = true,
                            _ => {}
                        }
                    }
                    if has_on {
                        return SqlContext::JoinCondition;
                    } else if has_table {
                        return SqlContext::JoinClause; // Might need alias or ON
                    }
                    return SqlContext::JoinClause;
                }
                "WHERE" | "AND" | "OR" => return SqlContext::WhereClause,
                "GROUP" => {
                    // Check for BY
                    if i + 1 < tokens.len()
                        && matches!(&tokens[i + 1], Token::Keyword(k) if k == "BY")
                    {
                        return SqlContext::GroupByClause;
                    }
                }
                "BY" => {
                    // Check if GROUP BY or ORDER BY
                    if i > 0 {
                        if let Token::Keyword(prev) = &tokens[i - 1] {
                            match prev.as_str() {
                                "GROUP" => return SqlContext::GroupByClause,
                                "ORDER" => return SqlContext::OrderByClause,
                                _ => {}
                            }
                        }
                    }
                }
                "HAVING" => return SqlContext::HavingClause,
                "ORDER" => {
                    // Could be ORDER BY or ORDER table SET...
                    if i + 1 < tokens.len()
                        && matches!(&tokens[i + 1], Token::Keyword(k) if k == "BY")
                    {
                        return SqlContext::OrderByClause;
                    }
                    // Check for ORDER table SET pattern
                    for token in tokens.iter().skip(i + 1) {
                        if matches!(token, Token::Keyword(k) if k == "ABOVE" || k == "BELOW") {
                            return SqlContext::OrderPosition;
                        }
                    }
                    return SqlContext::OrderPosition;
                }
                "SET" => return SqlContext::SetClause,
                "INSERT" | "INTO" => return SqlContext::InsertClause,
                "VALUES" => return SqlContext::ValuesClause,
                "CREATE" => return SqlContext::AfterCreate,
                "ALTER" => return SqlContext::AfterAlter,
                "DROP" => return SqlContext::AfterDrop,
                "PROPERTIES" => return SqlContext::PropertyDefinition,
                "MOVE" => return SqlContext::MoveTarget,
                "ABOVE" | "BELOW" => return SqlContext::OrderPosition,
                "TO" => {
                    // Check if in MOVE context
                    for j in (0..i).rev() {
                        if matches!(&tokens[j], Token::Keyword(k) if k == "MOVE") {
                            return SqlContext::MoveTarget;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Check for semicolon at end (new statement)
    if matches!(tokens.last(), Some(Token::Semicolon)) {
        return SqlContext::StatementStart;
    }

    SqlContext::Unknown
}

/// Check if we're inside a transaction block
fn check_in_transaction(tokens: &[Token]) -> bool {
    let mut in_tx = false;
    for token in tokens {
        if let Token::Keyword(kw) = token {
            match kw.as_str() {
                "BEGIN" => in_tx = true,
                "COMMIT" | "ROLLBACK" => in_tx = false,
                _ => {}
            }
        }
    }
    in_tx
}
