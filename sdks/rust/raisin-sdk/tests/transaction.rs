//! `Transaction`'s Drop contract: an uncommitted guard rolls back.

use raisin_sdk::testing::{with_mock, MockHost};
use raisin_sdk::Transaction;

fn began() -> MockHost {
    MockHost::new().expect("tx_begin", "[]", Ok("\"tx-1\"".to_string()))
}

#[test]
fn a_committed_transaction_does_not_roll_back() {
    let mock = began().expect("tx_commit", r#"["tx-1"]"#, Ok("true".to_string()));
    let (_, mock) = with_mock(mock, || {
        let tx = Transaction::begin().expect("begin");
        assert_eq!(tx.id(), "tx-1");
        tx.commit().expect("commit");
    });
    let methods: Vec<&str> = mock.calls().iter().map(|c| c.method.as_str()).collect();
    assert_eq!(methods, vec!["tx_begin", "tx_commit"]);
}

#[test]
fn dropping_an_uncommitted_transaction_rolls_it_back() {
    let mock = began().expect("tx_rollback", r#"["tx-1"]"#, Ok("true".to_string()));
    let (_, mock) = with_mock(mock, || {
        let _tx = Transaction::begin().expect("begin");
        // no commit: the guard goes out of scope here
    });
    let methods: Vec<&str> = mock.calls().iter().map(|c| c.method.as_str()).collect();
    assert_eq!(methods, vec!["tx_begin", "tx_rollback"]);
}

#[test]
fn an_early_return_still_rolls_back() {
    let mock = began()
        .expect_any("tx_create", Err("validation failed".to_string()))
        .expect("tx_rollback", r#"["tx-1"]"#, Ok("true".to_string()));
    let (result, mock) = with_mock(mock, || -> raisin_sdk::Result<()> {
        let tx = Transaction::begin()?;
        raisin_sdk::tx::create(tx.id(), "content", "/people", serde_json::json!({}))?;
        tx.commit()
    });
    assert!(result.is_err());
    let methods: Vec<&str> = mock.calls().iter().map(|c| c.method.as_str()).collect();
    assert_eq!(methods, vec!["tx_begin", "tx_create", "tx_rollback"]);
}

#[test]
fn an_explicit_rollback_happens_once() {
    let mock = began().expect("tx_rollback", r#"["tx-1"]"#, Ok("true".to_string()));
    let (_, mock) = with_mock(mock, || {
        Transaction::begin()
            .expect("begin")
            .rollback()
            .expect("rollback");
    });
    assert_eq!(mock.calls().len(), 2);
}
