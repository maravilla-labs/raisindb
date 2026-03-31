//! Lightweight SQL tokenizer for context detection
//!
//! Provides a simplified tokenizer that identifies keywords, identifiers,
//! strings, numbers, and punctuation for determining cursor context.

/// SQL token for lightweight parsing
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token {
    Keyword(String),
    Identifier(String),
    String(String),
    Number(String),
    Dot,
    Comma,
    OpenParen,
    CloseParen,
    Semicolon,
    Operator(String),
    Star,
    Other(char),
}

/// Tokenize SQL text (simplified tokenizer for context detection)
pub(crate) fn tokenize(sql: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = sql.chars().peekable();

    while let Some(&c) = chars.peek() {
        match c {
            // Whitespace
            c if c.is_whitespace() => {
                chars.next();
            }
            // Single-line comment
            '-' => {
                chars.next();
                if chars.peek() == Some(&'-') {
                    chars.next();
                    // Skip to end of line
                    while let Some(&c) = chars.peek() {
                        chars.next();
                        if c == '\n' {
                            break;
                        }
                    }
                } else {
                    tokens.push(Token::Operator("-".to_string()));
                }
            }
            // String literal
            '\'' | '"' => {
                let quote = chars.next().unwrap();
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == quote {
                        // Check for escaped quote
                        if chars.peek() == Some(&quote) {
                            chars.next();
                            s.push(quote);
                        } else {
                            break;
                        }
                    } else {
                        s.push(c);
                    }
                }
                tokens.push(Token::String(s));
            }
            // Identifier or keyword
            c if c.is_alphabetic() || c == '_' => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_alphanumeric() || c == '_' {
                        s.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                // Check if it's a keyword
                let upper = s.to_uppercase();
                if is_sql_keyword(&upper) {
                    tokens.push(Token::Keyword(upper));
                } else {
                    tokens.push(Token::Identifier(s));
                }
            }
            // Number
            c if c.is_ascii_digit() => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() || c == '.' {
                        s.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Number(s));
            }
            // Punctuation
            '.' => {
                chars.next();
                tokens.push(Token::Dot);
            }
            ',' => {
                chars.next();
                tokens.push(Token::Comma);
            }
            '(' => {
                chars.next();
                tokens.push(Token::OpenParen);
            }
            ')' => {
                chars.next();
                tokens.push(Token::CloseParen);
            }
            ';' => {
                chars.next();
                tokens.push(Token::Semicolon);
            }
            '*' => {
                chars.next();
                tokens.push(Token::Star);
            }
            // Operators
            '=' | '<' | '>' | '!' | '+' | '/' | '%' | '&' | '|' | '^' => {
                let mut op = String::new();
                op.push(chars.next().unwrap());
                // Check for multi-char operators
                if let Some(&next) = chars.peek() {
                    if matches!(next, '=' | '>' | '<') {
                        op.push(chars.next().unwrap());
                    }
                }
                tokens.push(Token::Operator(op));
            }
            _ => {
                tokens.push(Token::Other(chars.next().unwrap()));
            }
        }
    }

    tokens
}

/// Check if a string is a SQL keyword
fn is_sql_keyword(s: &str) -> bool {
    matches!(
        s,
        "SELECT"
            | "FROM"
            | "WHERE"
            | "JOIN"
            | "LEFT"
            | "RIGHT"
            | "INNER"
            | "OUTER"
            | "CROSS"
            | "FULL"
            | "ON"
            | "AND"
            | "OR"
            | "NOT"
            | "IN"
            | "IS"
            | "NULL"
            | "LIKE"
            | "BETWEEN"
            | "EXISTS"
            | "CASE"
            | "WHEN"
            | "THEN"
            | "ELSE"
            | "END"
            | "AS"
            | "DISTINCT"
            | "ALL"
            | "GROUP"
            | "BY"
            | "HAVING"
            | "ORDER"
            | "ASC"
            | "DESC"
            | "NULLS"
            | "FIRST"
            | "LAST"
            | "LIMIT"
            | "OFFSET"
            | "UNION"
            | "INTERSECT"
            | "EXCEPT"
            | "INSERT"
            | "INTO"
            | "VALUES"
            | "UPDATE"
            | "SET"
            | "DELETE"
            | "CREATE"
            | "ALTER"
            | "DROP"
            | "NODETYPE"
            | "ARCHETYPE"
            | "ELEMENTTYPE"
            | "PROPERTIES"
            | "EXTENDS"
            | "BEGIN"
            | "COMMIT"
            | "ROLLBACK"
            | "TRANSACTION"
            | "WITH"
            | "RECURSIVE"
            | "MOVE"
            | "TO"
            | "ABOVE"
            | "BELOW"
            | "EXPLAIN"
            | "ANALYZE"
            | "TRUE"
            | "FALSE"
    )
}
