// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Reading JSON whose objects name each key once.
//!
//! JSON lets a text repeat a key, but says the result is then
//! "unpredictable" (RFC 8259, section 4): one reader keeps the first
//! value, another the last. `serde_json::Value` keeps the last, without a
//! word. For a policy that is unsafe: two parties could read two
//! different contracts in the same bytes. So a message, a constraint or a
//! datasets file with a repeated key is refused, the same way the typed
//! readers already refuse it ("duplicate field").

use std::fmt;

use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};

/// A JSON value in which no object has the same key twice, at any depth.
/// Reading one that does fails with "duplicate field".
#[derive(Debug, Clone, PartialEq)]
pub struct UniqueKeys(pub Value);

impl<'de> Deserialize<'de> for UniqueKeys {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(Reader).map(UniqueKeys)
    }
}

/// Builds the [`Value`]; it reads the values inside lists and objects
/// with itself, so that the check reaches every depth.
struct Reader;

impl<'de> DeserializeSeed<'de> for Reader {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Reader {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E: Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E: Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_f64<E: Error>(self, v: f64) -> Result<Value, E> {
        // JSON has no NaN or infinity; serde_json reads those as null too
        Ok(Number::from_f64(v).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E: Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E: Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E: Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E: Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut list: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = list.next_element_seed(Reader)? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Value, A::Error> {
        let mut fields = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if fields.contains_key(&key) {
                return Err(A::Error::custom(format!("duplicate field `{key}`")));
            }
            let value = object.next_value_seed(Reader)?;
            fields.insert(key, value);
        }
        Ok(Value::Object(fields))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_text_without_repeated_keys_reads_as_serde_json_reads_it() {
        let text = r#"{"a": [1, -2, 1.5, true, null, "x", {"b": {}}], "c": 18446744073709551615}"#;
        let plain: Value = serde_json::from_str(text).unwrap();
        let strict: UniqueKeys = serde_json::from_str(text).unwrap();
        assert_eq!(strict.0, plain);
        // and from a value already read
        let again: UniqueKeys = serde_json::from_value(plain.clone()).unwrap();
        assert_eq!(again.0, plain);
    }

    #[test]
    fn a_repeated_key_is_refused_by_name_at_any_depth() {
        for text in [
            r#"{"a": 1, "a": 2}"#,
            r#"{"a": 1, "a": 1}"#,
            r#"{"x": [{"y": {"a": 1, "a": 2}}]}"#,
        ] {
            let error = serde_json::from_str::<UniqueKeys>(text).unwrap_err();
            assert!(error.to_string().contains("duplicate field `a`"), "{error}");
        }
        // the same key in two objects is not a repeat
        assert!(serde_json::from_str::<UniqueKeys>(r#"[{"a": 1}, {"a": 2}]"#).is_ok());
    }
}
