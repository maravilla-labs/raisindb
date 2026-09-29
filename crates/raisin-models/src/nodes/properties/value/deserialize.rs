// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! `Deserialize` for [`PropertyValue`]: the untagged ladder, without its cost.
//!
//! # Why this is hand-written
//!
//! `PropertyValue` used `#[derive(Deserialize)] #[serde(untagged)]`. The derive
//! buffers the value and then tries every variant in declaration order until
//! one accepts it — and several variants are expensive ways to say no:
//!
//! - `Reference` and `Url` deserialize the WHOLE subtree into a
//!   `serde_json::Value` before looking at a single key;
//! - `Element` deserializes every field of the map as a `PropertyValue` and
//!   only then reports the missing `element_type`, after which `Object`
//!   deserializes the same fields again.
//!
//! Every nested object therefore paid for its subtree several times over, and
//! the factor compounded per nesting level. Measured on a real page (a 10 KB
//! MessagePack node of content blocks): 1.0 ms to decode, against 7 µs for the
//! raw MessagePack walk; a 170 KB resolved page took 268 ms. Every node read —
//! SQL scans, REST reads, RESOLVE(), function host calls — paid it. With this
//! implementation the same two decode in ~50 µs and ~1 ms.
//!
//! # What this does instead, and why it cannot change a result
//!
//! The value is buffered ONCE into [`Buf`] and classified by walking the same
//! ladder in the same order. Each typed variant is still decided by that type's
//! own deserializer, run over the buffer. The only addition is a GATE in front
//! of the expensive attempts, and every gate is a condition the variant's own
//! deserializer requires, so skipping the attempt when the gate fails skips an
//! attempt that could only have failed:
//!
//! | variant   | gate (necessary for its deserializer to succeed)            |
//! |-----------|--------------------------------------------------------------|
//! | Reference | a map with `raisin:ref` and `raisin:workspace`              |
//! | Url       | a map with `raisin:url`                                      |
//! | Resource  | a map with `uuid`, `created_at`, `updated_at`; or a sequence |
//! | Composite | a map with `uuid` and `items`; or a sequence                |
//! | Element   | a map with `element_type`                                    |
//! | Vector    | a sequence of numbers only                                   |
//! | Geometry  | a map with `type`; or a sequence                             |
//!
//! `Array` and `Object`, the fallbacks, classify their children straight from
//! the buffer instead of re-buffering them. `tests_deserialize.rs` checks the
//! result against the derived ladder over MessagePack and JSON corpora.

use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

use serde::de::{
    self, value::MapDeserializer, value::SeqDeserializer, DeserializeSeed, Deserializer,
    IntoDeserializer, Visitor,
};
use serde::Deserialize;

use crate::nodes::properties::utils::{
    deserialize_guarded_string, deserialize_raisin_reference, deserialize_raisin_url,
    deserialize_tagged_decimal,
};
use crate::timestamp::StorageTimestamp;

use super::domain_types::Resource;
use super::element::{Composite, Element};
use super::geojson::GeoJson;
use super::PropertyValue;

/// A buffered serde value — the equivalent of serde's private `Content`.
#[derive(Debug, Clone)]
pub(super) enum Buf {
    Unit,
    None,
    Some(Box<Buf>),
    Newtype(Box<Buf>),
    Bool(bool),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
    Char(char),
    Str(String),
    Bytes(Vec<u8>),
    Seq(Vec<Buf>),
    Map(Vec<(Buf, Buf)>),
}

impl Buf {
    fn has_key(&self, key: &str) -> bool {
        match self {
            Buf::Map(entries) => entries
                .iter()
                .any(|(k, _)| matches!(k, Buf::Str(s) if s == key)),
            _ => false,
        }
    }

    fn is_number(&self) -> bool {
        matches!(self, Buf::U64(_) | Buf::I64(_) | Buf::F32(_) | Buf::F64(_))
    }

    fn unexpected(&self) -> de::Unexpected<'_> {
        match self {
            Buf::Unit => de::Unexpected::Unit,
            Buf::None | Buf::Some(_) => de::Unexpected::Option,
            Buf::Newtype(_) => de::Unexpected::NewtypeStruct,
            Buf::Bool(b) => de::Unexpected::Bool(*b),
            Buf::U64(n) => de::Unexpected::Unsigned(*n),
            Buf::I64(n) => de::Unexpected::Signed(*n),
            Buf::F32(n) => de::Unexpected::Float(f64::from(*n)),
            Buf::F64(n) => de::Unexpected::Float(*n),
            Buf::Char(c) => de::Unexpected::Char(*c),
            Buf::Str(s) => de::Unexpected::Str(s),
            Buf::Bytes(b) => de::Unexpected::Bytes(b),
            Buf::Seq(_) => de::Unexpected::Seq,
            Buf::Map(_) => de::Unexpected::Map,
        }
    }
}

impl<'de> Deserialize<'de> for Buf {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(BufVisitor)
    }
}

struct BufVisitor;

impl<'de> Visitor<'de> for BufVisitor {
    type Value = Buf;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Buf, E> {
        Ok(Buf::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Buf, E> {
        Ok(Buf::I64(v))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Buf, E> {
        Ok(Buf::U64(v))
    }
    fn visit_f32<E>(self, v: f32) -> Result<Buf, E> {
        Ok(Buf::F32(v))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Buf, E> {
        Ok(Buf::F64(v))
    }
    fn visit_char<E>(self, v: char) -> Result<Buf, E> {
        Ok(Buf::Char(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<Buf, E> {
        Ok(Buf::Str(v.to_owned()))
    }
    fn visit_string<E>(self, v: String) -> Result<Buf, E> {
        Ok(Buf::Str(v))
    }
    fn visit_bytes<E>(self, v: &[u8]) -> Result<Buf, E> {
        Ok(Buf::Bytes(v.to_vec()))
    }
    fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Buf, E> {
        Ok(Buf::Bytes(v))
    }
    fn visit_unit<E>(self) -> Result<Buf, E> {
        Ok(Buf::Unit)
    }
    fn visit_none<E>(self) -> Result<Buf, E> {
        Ok(Buf::None)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Buf, D::Error> {
        Ok(Buf::Some(Box::new(Buf::deserialize(d)?)))
    }
    fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<Buf, D::Error> {
        Ok(Buf::Newtype(Box::new(Buf::deserialize(d)?)))
    }
    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Buf, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(4096));
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Buf::Seq(items))
    }
    fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Buf, A::Error> {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0).min(4096));
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(Buf::Map(entries))
    }
}

/// A `Deserializer` over a borrowed [`Buf`], mirroring serde's
/// `ContentRefDeserializer` — including reporting `is_human_readable() == true`
/// whatever the source format was, which the variant deserializers rely on
/// (see `StorageTimestamp`'s visitor).
pub(super) struct BufRef<'a, E> {
    buf: &'a Buf,
    err: PhantomData<E>,
}

impl<'a, E> BufRef<'a, E> {
    fn new(buf: &'a Buf) -> Self {
        BufRef {
            buf,
            err: PhantomData,
        }
    }
}

impl<'a, E: de::Error> IntoDeserializer<'a, E> for &'a Buf {
    type Deserializer = BufRef<'a, E>;
    fn into_deserializer(self) -> BufRef<'a, E> {
        BufRef::new(self)
    }
}

impl<'de, 'a: 'de, E: de::Error> Deserializer<'de> for BufRef<'a, E> {
    type Error = E;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.buf {
            Buf::Unit => visitor.visit_unit(),
            Buf::None => visitor.visit_none(),
            Buf::Some(v) => visitor.visit_some(BufRef::new(v)),
            Buf::Newtype(v) => visitor.visit_newtype_struct(BufRef::new(v)),
            Buf::Bool(v) => visitor.visit_bool(*v),
            Buf::U64(v) => visitor.visit_u64(*v),
            Buf::I64(v) => visitor.visit_i64(*v),
            Buf::F32(v) => visitor.visit_f32(*v),
            Buf::F64(v) => visitor.visit_f64(*v),
            Buf::Char(v) => visitor.visit_char(*v),
            Buf::Str(v) => visitor.visit_borrowed_str(v),
            Buf::Bytes(v) => visitor.visit_borrowed_bytes(v),
            Buf::Seq(items) => {
                let mut seq = SeqDeserializer::new(items.iter());
                let value = visitor.visit_seq(&mut seq)?;
                seq.end()?;
                Ok(value)
            }
            Buf::Map(entries) => {
                let mut map = MapDeserializer::new(entries.iter().map(|(k, v)| (k, v)));
                let value = visitor.visit_map(&mut map)?;
                map.end()?;
                Ok(value)
            }
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.buf {
            Buf::None | Buf::Unit => visitor.visit_none(),
            Buf::Some(v) => visitor.visit_some(BufRef::new(v)),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, E> {
        match self.buf {
            Buf::Newtype(v) => visitor.visit_newtype_struct(BufRef::new(v)),
            _ => visitor.visit_newtype_struct(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        match self.buf {
            Buf::Unit => visitor.visit_unit(),
            Buf::Seq(items) if items.is_empty() => visitor.visit_unit(),
            other => Err(de::Error::invalid_type(other.unexpected(), &visitor)),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, E> {
        match self.buf {
            Buf::Str(variant) => visitor.visit_enum(variant.as_str().into_deserializer()),
            Buf::Map(entries) if entries.len() == 1 => visitor.visit_enum(BufEnum {
                variant: &entries[0].0,
                value: Some(&entries[0].1),
                err: PhantomData,
            }),
            other => Err(de::Error::invalid_type(other.unexpected(), &"enum")),
        }
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, E> {
        visitor.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit_struct seq tuple tuple_struct map struct identifier
    }
}

struct BufEnum<'a, E> {
    variant: &'a Buf,
    value: Option<&'a Buf>,
    err: PhantomData<E>,
}

impl<'de, 'a: 'de, E: de::Error> de::EnumAccess<'de> for BufEnum<'a, E> {
    type Error = E;
    type Variant = BufVariant<'a, E>;

    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, BufVariant<'a, E>), E> {
        let variant = seed.deserialize(BufRef::<E>::new(self.variant))?;
        Ok((
            variant,
            BufVariant {
                value: self.value,
                err: PhantomData,
            },
        ))
    }
}

struct BufVariant<'a, E> {
    value: Option<&'a Buf>,
    err: PhantomData<E>,
}

impl<'de, 'a: 'de, E: de::Error> de::VariantAccess<'de> for BufVariant<'a, E> {
    type Error = E;

    fn unit_variant(self) -> Result<(), E> {
        match self.value {
            None | Some(Buf::Unit) => Ok(()),
            Some(other) => Err(de::Error::invalid_type(other.unexpected(), &"unit variant")),
        }
    }

    fn newtype_variant_seed<S: DeserializeSeed<'de>>(self, seed: S) -> Result<S::Value, E> {
        match self.value {
            Some(v) => seed.deserialize(BufRef::new(v)),
            None => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"newtype variant",
            )),
        }
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, E> {
        match self.value {
            Some(v @ Buf::Seq(_)) => BufRef::new(v).deserialize_any(visitor),
            _ => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"tuple variant",
            )),
        }
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, E> {
        match self.value {
            Some(v @ (Buf::Map(_) | Buf::Seq(_))) => BufRef::new(v).deserialize_any(visitor),
            _ => Err(de::Error::invalid_type(
                de::Unexpected::UnitVariant,
                &"struct variant",
            )),
        }
    }
}

/// The error the variant attempts run with.
///
/// Their errors are discarded — an untagged enum only asks "did it match" — so
/// this one records nothing. That matters more than it looks: serde builds a
/// mismatch message with `format_args!`, and `invalid_type` on a string quotes
/// the whole string with `Debug` escaping. With a real error type every failed
/// attempt on a block of body text formatted that text, which profiled as the
/// largest single cost of decoding a page once the re-parsing was gone.
#[derive(Debug)]
struct Attempt;

impl fmt::Display for Attempt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("variant does not match")
    }
}

impl std::error::Error for Attempt {}

impl de::Error for Attempt {
    fn custom<T: fmt::Display>(_msg: T) -> Self {
        Attempt
    }
}

fn attempt<T>(result: Result<T, impl Sized>) -> Option<T> {
    result.ok()
}

/// Classify a buffered value, walking the ladder in declaration order.
///
/// Returns `None` when no variant matches, exactly where the derived untagged
/// enum reported "data did not match any variant".
pub(super) fn classify(buf: &Buf) -> Option<PropertyValue> {
    let d = || BufRef::<Attempt>::new(buf);

    // Null, Boolean, Integer, Float: primitive visitors accept exactly these.
    match buf {
        Buf::Unit | Buf::None => return Some(PropertyValue::Null),
        Buf::Bool(b) => return Some(PropertyValue::Boolean(*b)),
        Buf::I64(n) => return Some(PropertyValue::Integer(*n)),
        Buf::U64(n) => {
            return Some(match i64::try_from(*n) {
                Ok(n) => PropertyValue::Integer(n),
                Err(_) => PropertyValue::Float(*n as f64),
            })
        }
        Buf::F32(n) => return Some(PropertyValue::Float(f64::from(*n))),
        Buf::F64(n) => return Some(PropertyValue::Float(*n)),
        _ => {}
    }

    let is_map = matches!(buf, Buf::Map(_));
    let is_seq = matches!(buf, Buf::Seq(_));
    // serde's default `visit_char` forwards to `visit_str`.
    let is_str = matches!(buf, Buf::Str(_) | Buf::Char(_));

    // Date: an RFC3339 string, or the `[nanoseconds]` tuple.
    if is_str || is_seq {
        if let Some(v) = attempt(StorageTimestamp::deserialize(d())) {
            return Some(PropertyValue::Date(v));
        }
    }
    // Decimal and String: a string, or their one-key tag maps. Both visitors
    // give up on a map at its first key, so they are not gated further.
    if is_str || is_map {
        if let Some(v) = attempt(deserialize_tagged_decimal(d())) {
            return Some(PropertyValue::Decimal(v));
        }
        if let Buf::Str(v) = buf {
            // What the guarded-string visitor returns for a bare string.
            return Some(PropertyValue::String(v.clone()));
        }
        if let Some(v) = attempt(deserialize_guarded_string(d())) {
            return Some(PropertyValue::String(v));
        }
    }
    if buf.has_key("raisin:ref") && buf.has_key("raisin:workspace") {
        if let Some(v) = attempt(deserialize_raisin_reference(d())) {
            return Some(PropertyValue::Reference(v));
        }
    }
    if buf.has_key("raisin:url") {
        if let Some(v) = attempt(deserialize_raisin_url(d())) {
            return Some(PropertyValue::Url(v));
        }
    }
    // A derived struct also accepts its fields as a sequence (MessagePack's
    // compact struct form), so sequences always get the attempt.
    if is_seq || (buf.has_key("uuid") && buf.has_key("created_at") && buf.has_key("updated_at")) {
        if let Some(v) = attempt(Resource::deserialize(d())) {
            return Some(PropertyValue::Resource(v));
        }
    }
    if is_seq || (buf.has_key("uuid") && buf.has_key("items")) {
        if let Some(v) = attempt(Composite::deserialize(d())) {
            return Some(PropertyValue::Composite(v));
        }
    }
    if buf.has_key("element_type") {
        if let Some(v) = element_from_buf(buf) {
            return Some(PropertyValue::Element(v));
        }
    }
    if let Buf::Seq(items) = buf {
        if items.iter().all(Buf::is_number) {
            if let Some(v) = attempt(Vec::<f32>::deserialize(d())) {
                return Some(PropertyValue::Vector(v));
            }
        }
    }
    // An internally tagged enum also reads the sequence form, tag first.
    let geometry_shaped = is_seq || buf.has_key("type");
    if geometry_shaped {
        if let Some(v) = attempt(GeoJson::deserialize(d())) {
            return Some(PropertyValue::Geometry(v));
        }
    }

    match buf {
        Buf::Seq(items) => items
            .iter()
            .map(classify)
            .collect::<Option<Vec<_>>>()
            .map(PropertyValue::Array),
        Buf::Map(entries) => {
            let mut map = HashMap::with_capacity(entries.len());
            for (k, v) in entries {
                // What serde's `String` visitor accepts as a key.
                let key = match k {
                    Buf::Str(s) => s.clone(),
                    Buf::Char(c) => c.to_string(),
                    Buf::Bytes(b) => String::from_utf8(b.clone()).ok()?,
                    _ => return None,
                };
                map.insert(key, classify(v)?);
            }
            Some(PropertyValue::Object(map))
        }
        _ => None,
    }
}

/// What serde's `String` visitor accepts: a string, a char, or UTF-8 bytes.
fn string_from_buf(buf: &Buf) -> Option<String> {
    match buf {
        Buf::Str(s) => Some(s.clone()),
        Buf::Char(c) => Some(c.to_string()),
        Buf::Bytes(b) => String::from_utf8(b.clone()).ok(),
        _ => None,
    }
}

/// `Element`'s own deserializer (`element.rs`), read straight off the buffer.
///
/// Elements are where page content lives, so going through
/// `Element::deserialize` would re-buffer every block's fields once more. The
/// rules are the visitor's, line for line: `element_type` and `uuid` are
/// strings and may not repeat, `content` is held back and unwrapped only when
/// it is the sole field (the legacy nested form), everything else is a field.
/// `tests_deserialize.rs` holds the two to the same answers.
fn element_from_buf(buf: &Buf) -> Option<Element> {
    let Buf::Map(entries) = buf else {
        return None;
    };
    let mut element_type: Option<String> = None;
    let mut uuid: Option<String> = None;
    let mut content = HashMap::new();
    let mut nested_content: Option<PropertyValue> = None;

    for (k, v) in entries {
        let key = string_from_buf(k)?;
        match key.as_str() {
            "element_type" => {
                if element_type.is_some() {
                    return None;
                }
                element_type = Some(string_from_buf(v)?);
            }
            "uuid" => {
                if uuid.is_some() {
                    return None;
                }
                uuid = Some(string_from_buf(v)?);
            }
            "content" => nested_content = Some(classify(v)?),
            _ => {
                content.insert(key, classify(v)?);
            }
        }
    }

    if let Some(nested) = nested_content {
        match nested {
            PropertyValue::Object(obj) if content.is_empty() => content = obj,
            other => {
                content.insert("content".to_string(), other);
            }
        }
    }

    Some(Element {
        uuid: uuid.unwrap_or_default(),
        element_type: element_type?,
        content,
    })
}

impl<'de> Deserialize<'de> for PropertyValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let buf = Buf::deserialize(deserializer)?;
        classify(&buf).ok_or_else(|| {
            de::Error::custom("data did not match any variant of untagged enum PropertyValue")
        })
    }
}
