//! The four things retrieval and answering need from the host, behind a trait.
//!
//! Production goes through `raisin_sdk` (the WIT gateway). The unit tests hand
//! in a closure-driven fake instead of scripting `MockHost` call by call: the
//! interesting assertions are about WHICH SQL was issued and WHICH prompts were
//! sent, and a strictly ordered script would make every test restate the whole
//! conversation.

use serde_json::Value;

/// Host operations, as plain `Result<_, String>` so a failure keeps the host's
/// own message.
pub trait Backend {
    /// `raisin.sql.query` — row-level security applies, exactly as for the
    /// JavaScript functions this replaces. Returns the rows.
    fn sql(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>, String>;
    /// `raisin.ai.completion`.
    fn completion(&self, request: &Value) -> Result<Value, String>;
    /// `raisin.ai.getDefaultModel('chat')`.
    fn default_chat_model(&self) -> Option<String>;
    /// `raisin.functions.call` (the plain function-to-function form).
    fn call_function(&self, path: &str, args: &Value) -> Result<Value, String>;
    /// A log line (goes to the execution's logs).
    fn log(&self, message: &str);
}

/// The real host.
pub struct Host;

impl Backend for Host {
    fn sql(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>, String> {
        let out = raisin_sdk::sql::query(sql, params).map_err(|e| e.message().to_string())?;
        Ok(rows_of(out))
    }

    fn completion(&self, request: &Value) -> Result<Value, String> {
        raisin_sdk::ai::completion(request).map_err(|e| e.message().to_string())
    }

    fn default_chat_model(&self) -> Option<String> {
        match raisin_sdk::ai::get_default_model("chat") {
            Ok(Some(Value::String(s))) if !s.trim().is_empty() => Some(s),
            _ => None,
        }
    }

    fn call_function(&self, path: &str, args: &Value) -> Result<Value, String> {
        raisin_sdk::functions::call(path, args).map_err(|e| e.message().to_string())
    }

    fn log(&self, message: &str) {
        raisin_sdk::log::info(message);
    }
}

/// The rows of a query answer: an array (what production returns), or the
/// `{ rows: [...] }` shape some surfaces use.
pub fn rows_of(out: Value) -> Vec<Value> {
    match out {
        Value::Array(rows) => rows,
        Value::Object(mut o) => match o.remove("rows") {
            Some(Value::Array(rows)) => rows,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

#[cfg(test)]
pub mod fake {
    //! A backend whose every answer is a closure, recording every call.

    use super::Backend;
    use serde_json::Value;
    use std::cell::RefCell;

    type SqlFn = Box<dyn Fn(&str, &[Value]) -> Result<Vec<Value>, String>>;
    type CompletionFn = Box<dyn Fn(&Value) -> Result<Value, String>>;
    type CallFn = Box<dyn Fn(&str, &Value) -> Result<Value, String>>;

    pub struct Fake {
        pub sql_fn: SqlFn,
        pub completion_fn: CompletionFn,
        pub call_fn: CallFn,
        pub model: Option<String>,
        pub sql_log: RefCell<Vec<(String, Vec<Value>)>>,
        pub completions: RefCell<Vec<Value>>,
        pub calls: RefCell<Vec<(String, Value)>>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self {
                sql_fn: Box::new(|_, _| Ok(Vec::new())),
                completion_fn: Box::new(|_| Ok(serde_json::json!({ "content": "" }))),
                call_fn: Box::new(|_, _| Ok(serde_json::json!({ "nodes": [] }))),
                model: Some("openai:gpt-4o".to_string()),
                sql_log: RefCell::new(Vec::new()),
                completions: RefCell::new(Vec::new()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl Fake {
        pub fn sqls(&self) -> Vec<String> {
            self.sql_log
                .borrow()
                .iter()
                .map(|(s, _)| s.clone())
                .collect()
        }
    }

    impl Backend for Fake {
        fn sql(&self, sql: &str, params: &[Value]) -> Result<Vec<Value>, String> {
            self.sql_log
                .borrow_mut()
                .push((sql.to_string(), params.to_vec()));
            (self.sql_fn)(sql, params)
        }
        fn completion(&self, request: &Value) -> Result<Value, String> {
            self.completions.borrow_mut().push(request.clone());
            (self.completion_fn)(request)
        }
        fn default_chat_model(&self) -> Option<String> {
            self.model.clone()
        }
        fn call_function(&self, path: &str, args: &Value) -> Result<Value, String> {
            self.calls
                .borrow_mut()
                .push((path.to_string(), args.clone()));
            (self.call_fn)(path, args)
        }
        fn log(&self, _message: &str) {}
    }
}
