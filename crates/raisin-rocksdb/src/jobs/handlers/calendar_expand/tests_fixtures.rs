//! Node fixtures shared by every expander test.

use std::collections::HashMap;

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

use super::EVENT_TYPE;

pub(super) const TENANT: &str = "default";
pub(super) const WS: &str = "default";
pub(super) const ZURICH: &str = "Europe/Zurich";

pub(super) fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// A weekly-Tuesday-09:00 master, anchored on 2026-03-10 (still CET).
pub(super) fn weekly_master(id: &str, external: Option<&str>) -> Node {
    let mut props: HashMap<String, PropertyValue> = HashMap::new();
    props.insert("title".into(), s("Standup"));
    props.insert("recurrence_type".into(), s("series_master"));
    props.insert(
        "recurrence".into(),
        PropertyValue::Array(vec![s("RRULE:FREQ=WEEKLY;BYDAY=TU")]),
    );
    props.insert("timezone".into(), s(ZURICH));
    // 09:00 Europe/Zurich on 2026-03-10 is 08:00Z (CET, UTC+1).
    props.insert("start_utc".into(), s("2026-03-10T08:00:00Z"));
    props.insert("end_utc".into(), s("2026-03-10T08:30:00Z"));
    props.insert("start_local".into(), s("2026-03-10T09:00:00"));
    props.insert("status".into(), s("confirmed"));
    if let Some(ext) = external {
        props.insert("__external_id".into(), s(ext));
    }
    Node {
        id: id.to_string(),
        node_type: EVENT_TYPE.to_string(),
        name: id.to_string(),
        path: format!("/calendar/{id}"),
        workspace: Some(WS.to_string()),
        properties: props,
        ..Default::default()
    }
}

pub(super) fn exception_node(
    id: &str,
    master_external: &str,
    original_start_utc: &str,
    status: &str,
) -> Node {
    let mut props: HashMap<String, PropertyValue> = HashMap::new();
    props.insert("title".into(), s("Standup (moved)"));
    props.insert("recurrence_type".into(), s("exception"));
    props.insert("series_master_external_id".into(), s(master_external));
    props.insert("original_start_utc".into(), s(original_start_utc));
    props.insert("status".into(), s(status));
    props.insert("start_utc".into(), s("2026-03-17T10:00:00Z"));
    props.insert("timezone".into(), s(ZURICH));
    Node {
        id: id.to_string(),
        node_type: EVENT_TYPE.to_string(),
        name: id.to_string(),
        path: format!("/calendar/{id}"),
        workspace: Some(WS.to_string()),
        properties: props,
        ..Default::default()
    }
}
