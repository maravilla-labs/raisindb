//! `OpType`'s serde: the derived encoding, plus a catch-all for variants this
//! binary does not know (plan Phase 11, Operational test gates).
//!
//! The derive (`#[serde(remote = "Self")]`) produces inherent
//! `OpType::serialize` / `OpType::deserialize`; the trait impls below wrap
//! them. Decoding reads the variant TAG first, then replays it into the
//! derived visitor together with the untouched variant content — no
//! buffering, so every format keeps its own `is_human_readable` and every
//! known variant decodes exactly as before. Only when the derived visitor
//! rejects the tag (`unknown_variant`) is the content read as an opaque
//! `rmpv::Value` and kept in [`OpType::Unknown`].
//!
//! Serialization writes `Unknown` back as the same one-entry map
//! `{tag: payload}` every externally tagged struct/newtype variant uses, so
//! an op a node persisted or forwarded without understanding it is still the
//! op a newer node can decode.

use super::OpType;
use serde::de::{
    self, value::StrDeserializer, value::U64Deserializer, DeserializeSeed, Deserializer,
    EnumAccess, IntoDeserializer, VariantAccess, Visitor,
};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;

impl Serialize for OpType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            OpType::Unknown { tag, payload } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(tag, payload)?;
                map.end()
            }
            known => OpType::serialize(known, serializer),
        }
    }
}

impl<'de> Deserialize<'de> for OpType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_enum("OpType", &[], LenientVisitor)
    }
}

/// A variant tag as the format presents it: by name (self-describing
/// formats) or by index.
enum Tag {
    Name(String),
    Index(u64),
}

impl<'de> Deserialize<'de> for Tag {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TagVisitor;
        impl<'de> Visitor<'de> for TagVisitor {
            type Value = Tag;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an operation type tag")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Tag, E> {
                Ok(Tag::Name(v.to_string()))
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Tag, E> {
                Ok(Tag::Name(String::from_utf8_lossy(v).into_owned()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Tag, E> {
                Ok(Tag::Index(v))
            }
        }
        deserializer.deserialize_identifier(TagVisitor)
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Tag::Name(name) => f.write_str(name),
            Tag::Index(index) => write!(f, "#{index}"),
        }
    }
}

struct LenientVisitor;

impl<'de> Visitor<'de> for LenientVisitor {
    type Value = OpType;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an operation type")
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<OpType, A::Error> {
        let (tag, access): (Tag, A::Variant) = data.variant()?;
        let mut slot = Some(access);
        let decoded = OpType::deserialize(Replay {
            tag: &tag,
            slot: &mut slot,
        });
        match (decoded, slot) {
            (Ok(op), _) => Ok(op),
            // The derived visitor rejected the TAG (the content was never
            // handed out): a variant from a newer peer.
            (Err(_), Some(access)) => {
                let payload = access.newtype_variant::<rmpv::Value>()?;
                tracing::warn!(
                    op_type = %tag,
                    "decoded an operation type this binary does not know; it will be skipped"
                );
                Ok(OpType::Unknown {
                    tag: tag.to_string(),
                    payload,
                })
            }
            // A known variant whose content is malformed.
            (Err(e), None) => Err(e),
        }
    }
}

/// Re-presents the already-read tag and the untouched content to the derived
/// `OpType` visitor.
struct Replay<'a, A> {
    tag: &'a Tag,
    slot: &'a mut Option<A>,
}

impl<'de, 'a, A: VariantAccess<'de>> Deserializer<'de> for Replay<'a, A> {
    type Error = A::Error;

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_enum(self)
    }

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        Err(de::Error::custom("an operation type is an enum"))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

impl<'de, 'a, A: VariantAccess<'de>> EnumAccess<'de> for Replay<'a, A> {
    type Error = A::Error;
    type Variant = A;

    fn variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<(S::Value, A), Self::Error> {
        let value = match self.tag {
            Tag::Name(name) => {
                let de: StrDeserializer<'_, A::Error> = name.as_str().into_deserializer();
                seed.deserialize(de)?
            }
            Tag::Index(index) => {
                let de: U64Deserializer<A::Error> = (*index).into_deserializer();
                seed.deserialize(de)?
            }
        };
        let access = self
            .slot
            .take()
            .ok_or_else(|| de::Error::custom("operation content already consumed"))?;
        Ok((value, access))
    }
}
