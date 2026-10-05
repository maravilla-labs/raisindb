//! A projected overlay decode (plan Phase 13d): one entry of a properties
//! overlay, for a reader that needs one field — the localized lookup reads
//! only the node's translated name and whether the overlay hides it.

use crate::repositories::nodes::helpers::is_tombstone;
use raisin_error::Result;
use raisin_models::translations::LocaleOverlay;

/// [`decode_overlay`] keeping only the entry at `pointer` of a properties
/// overlay — what a reader that needs one field (the localized lookup: the
/// node's translated name) decodes, instead of every translated value
/// (plan Phase 13d). The kept value goes through the same `PropertyValue`
/// decode as a full read, so it is the same value.
pub(crate) fn decode_overlay_keeping(value: &[u8], pointer: &str) -> Result<Option<LocaleOverlay>> {
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::translations::JsonPointer;
    use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
    use std::collections::HashMap;

    if is_tombstone(value) {
        return Ok(None);
    }
    /// The `data` map, keeping one entry.
    struct Kept(Option<PropertyValue>);
    struct KeptSeed<'p>(&'p str);
    impl<'de> serde::de::DeserializeSeed<'de> for KeptSeed<'_> {
        type Value = Kept;
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Kept, D::Error> {
            struct V<'p>(&'p str);
            impl<'de> Visitor<'de> for V<'_> {
                type Value = Kept;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("an overlay data map")
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<Kept, A::Error> {
                    let mut kept = None;
                    while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
                        if key == self.0 {
                            kept = Some(map.next_value::<PropertyValue>()?);
                        } else {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                    Ok(Kept(kept))
                }
            }
            d.deserialize_map(V(self.0))
        }
    }
    struct Overlay<'p>(&'p str);
    impl<'de> serde::de::DeserializeSeed<'de> for Overlay<'_> {
        type Value = (Option<String>, Option<Kept>);
        fn deserialize<D: Deserializer<'de>>(
            self,
            d: D,
        ) -> std::result::Result<Self::Value, D::Error> {
            struct V<'p>(&'p str);
            impl<'de> Visitor<'de> for V<'_> {
                type Value = (Option<String>, Option<Kept>);
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a locale overlay")
                }
                fn visit_map<A: MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> std::result::Result<Self::Value, A::Error> {
                    let (mut kind, mut data) = (None, None);
                    while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
                        match key.as_ref() {
                            "type" => kind = Some(map.next_value::<String>()?),
                            "data" => data = Some(map.next_value_seed(KeptSeed(self.0))?),
                            _ => {
                                map.next_value::<IgnoredAny>()?;
                            }
                        }
                    }
                    Ok((kind, data))
                }
            }
            d.deserialize_map(V(self.0))
        }
    }
    let error = |e: String| {
        raisin_error::Error::storage(format!("Failed to deserialize LocaleOverlay: {e}"))
    };
    let mut de = serde_json::Deserializer::from_slice(value);
    let (kind, data) = serde::de::DeserializeSeed::deserialize(Overlay(pointer), &mut de)
        .map_err(|e| error(e.to_string()))?;
    de.end().map_err(|e| error(e.to_string()))?;
    match (kind.as_deref(), data) {
        (Some("hidden"), _) => Ok(Some(LocaleOverlay::Hidden)),
        (Some("properties"), Some(Kept(kept))) => {
            let mut data = HashMap::new();
            if let Some(v) = kept {
                data.insert(JsonPointer::new(pointer), v);
            }
            Ok(Some(LocaleOverlay::Properties { data }))
        }
        (kind, _) => Err(error(format!("unexpected overlay {kind:?}"))),
    }
}
