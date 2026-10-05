//! A prepared statement's Execute hands the engine its SQL TEXT and the
//! portal's values — never a substituted text — so every Execute of one
//! statement is bound into the engine's one template for it (plan Phase 13d).

use super::portal_params;
use crate::extended_query::statement::RaisinStatement;
use bytes::Bytes;
use pgwire::api::portal::Portal;
use pgwire::api::stmt::StoredStatement;
use pgwire::api::Type;
use pgwire::messages::extendedquery::Bind;
use serde_json::json;
use std::sync::Arc;

/// A portal over `sql` bound to binary-format `values` (what pgwire decodes).
fn portal(sql: &str, types: Vec<Type>, values: Vec<Option<Bytes>>) -> Portal<RaisinStatement> {
    let statement = Arc::new(StoredStatement::new(
        "s1".to_string(),
        RaisinStatement::new(sql.to_string(), types.clone()),
        types,
    ));
    let bind = Bind::new(
        None,
        Some("s1".to_string()),
        vec![1], // binary
        values,
        vec![],
    );
    Portal::try_new(&bind, statement).expect("portal")
}

#[test]
fn each_execute_hands_over_the_statement_text_and_its_values() {
    let sql = "SELECT id FROM ws WHERE path = $1 AND properties->>'n'::String = $2";
    let text = |s: &str| Some(Bytes::from(s.to_string()));
    let int = |n: i32| Some(Bytes::copy_from_slice(&n.to_be_bytes()));
    for (path, n) in [("/a", 5), ("/b", 6)] {
        let portal = portal(sql, vec![Type::TEXT, Type::INT4], vec![text(path), int(n)]);
        // The same text every time: the engine's template key.
        assert_eq!(portal.statement.statement.sql, sql);
        assert_eq!(portal_params(&portal).unwrap(), vec![json!(path), json!(n)]);
    }
    let null = portal(sql, vec![Type::TEXT, Type::INT4], vec![None, int(1)]);
    assert_eq!(portal_params(&null).unwrap(), vec![json!(null), json!(1)]);
}
