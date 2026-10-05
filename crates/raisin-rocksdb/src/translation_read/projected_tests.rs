//! `decode_overlay_keeping` (plan Phase 13d): the full decode, filtered to one
//! entry — the same value, through the same `PropertyValue` decode.

use super::{decode_overlay, decode_overlay_keeping};
use raisin_models::translations::{JsonPointer, LocaleOverlay};

const NAME: &str = "/__node_name";

fn filtered(value: &[u8]) -> Option<LocaleOverlay> {
    decode_overlay(value).unwrap().map(|overlay| match overlay {
        LocaleOverlay::Properties { data } => LocaleOverlay::Properties {
            data: data
                .into_iter()
                .filter(|(k, _)| k == &JsonPointer::new(NAME))
                .collect(),
        },
        hidden => hidden,
    })
}

#[test]
fn keeps_exactly_what_a_full_decode_has_at_the_pointer() {
    let cases: Vec<&[u8]> = vec![
        br#"{"type":"properties","data":{"/title":{"raisin:ref":"x"},"/__node_name":"chaise","/n":[1,2.5,{"a":null}]}}"#,
        br#"{"data":{"/__node_name":"2024-01-01T00:00:00Z","/t":"x"},"type":"properties"}"#,
        br#"{"type":"properties","data":{"/title":"only"}}"#,
        br#"{"type":"properties","data":{"/__node_name":"a","/__node_name":"b"}}"#,
        br#"{"type":"hidden"}"#,
        b"T",
    ];
    for value in cases {
        assert_eq!(
            decode_overlay_keeping(value, NAME).unwrap(),
            filtered(value),
            "{}",
            String::from_utf8_lossy(value)
        );
    }
    assert!(decode_overlay_keeping(br#"{"type":"bogus"}"#, NAME).is_err());
    assert!(decode_overlay_keeping(br#"{"type":"properties""#, NAME).is_err());
}
