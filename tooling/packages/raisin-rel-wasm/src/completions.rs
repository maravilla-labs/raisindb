//! REL expression completion suggestions

use crate::types::CompletionItem;

/// Get all available methods for autocomplete
pub(crate) fn get_all_methods() -> Vec<CompletionItem> {
    vec![
        // Universal methods
        CompletionItem {
            label: "length".to_string(),
            kind: "method".to_string(),
            insert_text: "length()".to_string(),
            detail: Some("Get length (string, array, or object keys)".to_string()),
        },
        CompletionItem {
            label: "isEmpty".to_string(),
            kind: "method".to_string(),
            insert_text: "isEmpty()".to_string(),
            detail: Some("Check if empty or null".to_string()),
        },
        CompletionItem {
            label: "isNotEmpty".to_string(),
            kind: "method".to_string(),
            insert_text: "isNotEmpty()".to_string(),
            detail: Some("Check if not empty".to_string()),
        },

        // Polymorphic method
        CompletionItem {
            label: "contains".to_string(),
            kind: "method".to_string(),
            insert_text: "contains(${1:value})".to_string(),
            detail: Some("String: contains substring, Array: has element".to_string()),
        },

        // String methods
        CompletionItem {
            label: "startsWith".to_string(),
            kind: "method".to_string(),
            insert_text: "startsWith(${1:prefix})".to_string(),
            detail: Some("Check if starts with prefix".to_string()),
        },
        CompletionItem {
            label: "endsWith".to_string(),
            kind: "method".to_string(),
            insert_text: "endsWith(${1:suffix})".to_string(),
            detail: Some("Check if ends with suffix".to_string()),
        },
        CompletionItem {
            label: "toLowerCase".to_string(),
            kind: "method".to_string(),
            insert_text: "toLowerCase()".to_string(),
            detail: Some("Convert to lowercase".to_string()),
        },
        CompletionItem {
            label: "toUpperCase".to_string(),
            kind: "method".to_string(),
            insert_text: "toUpperCase()".to_string(),
            detail: Some("Convert to uppercase".to_string()),
        },
        CompletionItem {
            label: "trim".to_string(),
            kind: "method".to_string(),
            insert_text: "trim()".to_string(),
            detail: Some("Remove whitespace".to_string()),
        },
        CompletionItem {
            label: "substring".to_string(),
            kind: "method".to_string(),
            insert_text: "substring(${1:start}, ${2:end})".to_string(),
            detail: Some("Extract substring".to_string()),
        },

        // Array methods
        CompletionItem {
            label: "first".to_string(),
            kind: "method".to_string(),
            insert_text: "first()".to_string(),
            detail: Some("Get first element".to_string()),
        },
        CompletionItem {
            label: "last".to_string(),
            kind: "method".to_string(),
            insert_text: "last()".to_string(),
            detail: Some("Get last element".to_string()),
        },
        CompletionItem {
            label: "indexOf".to_string(),
            kind: "method".to_string(),
            insert_text: "indexOf(${1:element})".to_string(),
            detail: Some("Find element index (-1 if not found)".to_string()),
        },
        CompletionItem {
            label: "join".to_string(),
            kind: "method".to_string(),
            insert_text: "join(${1:separator})".to_string(),
            detail: Some("Join array into string".to_string()),
        },

        // Path methods
        CompletionItem {
            label: "parent".to_string(),
            kind: "method".to_string(),
            insert_text: "parent()".to_string(),
            detail: Some("Get parent path".to_string()),
        },
        CompletionItem {
            label: "ancestor".to_string(),
            kind: "method".to_string(),
            insert_text: "ancestor(${1:depth})".to_string(),
            detail: Some("Get ancestor at depth".to_string()),
        },
        CompletionItem {
            label: "ancestorOf".to_string(),
            kind: "method".to_string(),
            insert_text: "ancestorOf(${1:path})".to_string(),
            detail: Some("Check if ancestor of path".to_string()),
        },
        CompletionItem {
            label: "descendantOf".to_string(),
            kind: "method".to_string(),
            insert_text: "descendantOf(${1:path})".to_string(),
            detail: Some("Check if descendant of path".to_string()),
        },
        CompletionItem {
            label: "childOf".to_string(),
            kind: "method".to_string(),
            insert_text: "childOf(${1:path})".to_string(),
            detail: Some("Check if direct child of path".to_string()),
        },
        CompletionItem {
            label: "depth".to_string(),
            kind: "method".to_string(),
            insert_text: "depth()".to_string(),
            detail: Some("Get hierarchy depth".to_string()),
        },
    ]
}

/// Compute completions based on cursor position
pub(crate) fn compute_completions(expression: &str, cursor_offset: usize) -> Vec<CompletionItem> {
    let mut completions = Vec::new();

    // Get the text before cursor
    let prefix = if cursor_offset <= expression.len() {
        &expression[..cursor_offset]
    } else {
        expression
    };

    // Check what we're completing
    let trimmed = prefix.trim_end();

    // After a dot - suggest property access
    if trimmed.ends_with('.') {
        return completions;
    }

    // After 'input.' or 'context.' - could suggest common fields
    if trimmed.ends_with("input") || trimmed.ends_with("context") {
        completions.push(CompletionItem {
            label: ".".to_string(),
            kind: "operator".to_string(),
            insert_text: ".".to_string(),
            detail: Some("Property access".to_string()),
        });
        return completions;
    }

    // At start or after operator - suggest everything
    let last_word = get_last_word(trimmed);

    add_function_completions(&last_word, &mut completions);
    add_context_completions(&last_word, &mut completions);
    add_keyword_completions(&last_word, &mut completions);
    add_operator_completions(&last_word, &mut completions);

    completions
}

/// Add function completions (contains, startsWith, endsWith)
fn add_function_completions(last_word: &str, completions: &mut Vec<CompletionItem>) {
    if "contains".starts_with(last_word) || last_word.is_empty() {
        completions.push(CompletionItem {
            label: "contains".to_string(),
            kind: "function".to_string(),
            insert_text: "contains(${1:field}, ${2:value})".to_string(),
            detail: Some("Test if string contains substring".to_string()),
        });
    }
    if "startsWith".starts_with(last_word) || last_word.is_empty() {
        completions.push(CompletionItem {
            label: "startsWith".to_string(),
            kind: "function".to_string(),
            insert_text: "startsWith(${1:field}, ${2:prefix})".to_string(),
            detail: Some("Test if string starts with prefix".to_string()),
        });
    }
    if "endsWith".starts_with(last_word) || last_word.is_empty() {
        completions.push(CompletionItem {
            label: "endsWith".to_string(),
            kind: "function".to_string(),
            insert_text: "endsWith(${1:field}, ${2:suffix})".to_string(),
            detail: Some("Test if string ends with suffix".to_string()),
        });
    }
}

/// Add context root completions (input, context)
fn add_context_completions(last_word: &str, completions: &mut Vec<CompletionItem>) {
    if "input".starts_with(last_word) || last_word.is_empty() {
        completions.push(CompletionItem {
            label: "input".to_string(),
            kind: "variable".to_string(),
            insert_text: "input.".to_string(),
            detail: Some("Access input data".to_string()),
        });
    }
    if "context".starts_with(last_word) || last_word.is_empty() {
        completions.push(CompletionItem {
            label: "context".to_string(),
            kind: "variable".to_string(),
            insert_text: "context.".to_string(),
            detail: Some("Access execution context".to_string()),
        });
    }
}

/// Add keyword completions (true, false, null)
fn add_keyword_completions(last_word: &str, completions: &mut Vec<CompletionItem>) {
    if "true".starts_with(last_word) || last_word.is_empty() {
        completions.push(CompletionItem {
            label: "true".to_string(),
            kind: "keyword".to_string(),
            insert_text: "true".to_string(),
            detail: Some("Boolean true".to_string()),
        });
    }
    if "false".starts_with(last_word) || last_word.is_empty() {
        completions.push(CompletionItem {
            label: "false".to_string(),
            kind: "keyword".to_string(),
            insert_text: "false".to_string(),
            detail: Some("Boolean false".to_string()),
        });
    }
    if "null".starts_with(last_word) || last_word.is_empty() {
        completions.push(CompletionItem {
            label: "null".to_string(),
            kind: "keyword".to_string(),
            insert_text: "null".to_string(),
            detail: Some("Null value".to_string()),
        });
    }
}

/// Add operator completions (only when no word is being typed)
fn add_operator_completions(last_word: &str, completions: &mut Vec<CompletionItem>) {
    if !last_word.is_empty() {
        return;
    }

    completions.push(CompletionItem {
        label: "==".to_string(),
        kind: "operator".to_string(),
        insert_text: "== ".to_string(),
        detail: Some("Equals".to_string()),
    });
    completions.push(CompletionItem {
        label: "!=".to_string(),
        kind: "operator".to_string(),
        insert_text: "!= ".to_string(),
        detail: Some("Not equals".to_string()),
    });
    completions.push(CompletionItem {
        label: "&&".to_string(),
        kind: "operator".to_string(),
        insert_text: "&& ".to_string(),
        detail: Some("Logical AND".to_string()),
    });
    completions.push(CompletionItem {
        label: "||".to_string(),
        kind: "operator".to_string(),
        insert_text: "|| ".to_string(),
        detail: Some("Logical OR".to_string()),
    });
}

/// Get the last word being typed
pub(crate) fn get_last_word(s: &str) -> String {
    let prefix = s.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    s[prefix.len()..].to_string()
}
