//! The generated wrapper layer, driven against `MockHost`.
//!
//! What is asserted here is the SEAM: argument encoding (a positional JSON
//! array), the registry `internal_name` each wrapper calls, and the decode
//! rules — including the defensive one, that an `Ok` body carrying an
//! `{"error": true, ...}` envelope is an error.

use raisin_sdk::testing::{with_mock, MockHost};
use raisin_sdk::Error;

#[test]
fn arguments_are_encoded_as_a_positional_json_array() {
    let mock = MockHost::new().expect(
        "nodes_get",
        r#"["content","/people/ada"]"#,
        Ok(r#"{"path":"/people/ada"}"#.to_string()),
    );
    let (node, mock) = with_mock(mock, || {
        raisin_sdk::nodes::get("content", "/people/ada").expect("get")
    });
    assert_eq!(node.expect("some")["path"], "/people/ada");
    assert_eq!(mock.calls()[0].method, "nodes_get");
    assert!(mock.unmet().is_empty());
}

#[test]
fn an_absent_optional_argument_is_json_null() {
    let mock = MockHost::new().expect(
        "nodes_getChildren",
        r#"["content","/people",null]"#,
        Ok("[]".to_string()),
    );
    let (children, _) = with_mock(mock, || {
        raisin_sdk::nodes::get_children("content", "/people", None).expect("children")
    });
    assert!(children.is_empty());
}

#[test]
fn null_decodes_to_none_for_an_optional_return() {
    let mock = MockHost::new().expect_any("nodes_get", Ok("null".to_string()));
    let (node, _) = with_mock(mock, || {
        raisin_sdk::nodes::get("content", "/missing").expect("get")
    });
    assert!(node.is_none());
}

#[test]
fn a_void_return_is_literal_true_on_the_wire() {
    let mock = MockHost::new().expect(
        "nodes_delete",
        r#"["content","/people/ada"]"#,
        Ok("true".to_string()),
    );
    let (out, _) = with_mock(mock, || raisin_sdk::nodes::delete("content", "/people/ada"));
    assert!(out.is_ok());
}

#[test]
fn a_host_err_becomes_an_error() {
    let mock = MockHost::new().expect_any("nodes_get", Err("no such workspace".to_string()));
    let (result, _) = with_mock(mock, || raisin_sdk::nodes::get("nope", "/x"));
    assert_eq!(result.unwrap_err(), Error::host("no such workspace"));
}

#[test]
fn an_ok_error_envelope_is_still_an_error() {
    let mock = MockHost::new().expect_any(
        "nodes_get",
        Ok(r#"{"error":true,"message":"row-level security denied the read"}"#.to_string()),
    );
    let (result, _) = with_mock(mock, || raisin_sdk::nodes::get("content", "/secret"));
    assert_eq!(
        result.unwrap_err(),
        Error::host("row-level security denied the read")
    );
}

#[test]
fn an_unscripted_call_is_an_error_not_a_default_answer() {
    let (result, _) = with_mock(MockHost::new(), || {
        raisin_sdk::nodes::get("content", "/people/ada")
    });
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("unexpected host call nodes_get"),
        "{message}"
    );
}

#[test]
fn the_typed_variant_deserialises_into_a_users_own_type() {
    #[derive(serde::Deserialize, PartialEq, Debug)]
    struct Person {
        name: String,
    }
    let mock = MockHost::new().expect_any("nodes_get", Ok(r#"{"name":"Ada"}"#.to_string()));
    let (person, _) = with_mock(mock, || {
        let person: Option<Person> =
            raisin_sdk::nodes::get_as("content", "/people/ada").expect("typed get");
        person
    });
    assert_eq!(
        person,
        Some(Person {
            name: "Ada".to_string()
        })
    );
}

#[test]
fn context_is_read_from_the_dedicated_import_not_the_gateway() {
    let mock = MockHost::new()
        .with_context(r#"{"tenant_id":"acme","repo_id":"main","branch":"draft","actor":"u-1"}"#);
    let (context, mock) = with_mock(mock, || {
        raisin_sdk::context::Context::get().expect("context")
    });
    assert_eq!(context.tenant_id(), Some("acme"));
    assert_eq!(context.actor(), Some("u-1"));
    assert!(
        mock.calls().is_empty(),
        "context must not cost a gateway call"
    );
}
