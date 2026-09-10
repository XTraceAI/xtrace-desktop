use crate::RuleId;
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};
use std::collections::BTreeMap;

// A normal map deserializer silently replaces repeated keys. Goldens must reject
// even identical duplicates while the input entries are still observable.
pub(super) struct Expected(pub BTreeMap<RuleId, Value>);

impl<'de> Deserialize<'de> for Expected {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ExpectedVisitor;
        impl<'de> Visitor<'de> for ExpectedVisitor {
            type Value = Expected;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an object of unique golden rule keys")
            }

            fn visit_map<M: MapAccess<'de>>(
                self,
                mut entries: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = entries.next_key::<RuleId>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate golden key {key}")));
                    }
                    values.insert(key, entries.next_value::<GoldenValue>()?.0);
                }
                Ok(Expected(values))
            }
        }
        deserializer.deserialize_map(ExpectedVisitor)
    }
}

// Decode every nested object through the same visitor. Value's default
// deserializer would silently discard duplicate fields inside an expectation.
struct GoldenValue(Value);

impl<'de> Deserialize<'de> for GoldenValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct GoldenVisitor;
        impl<'de> Visitor<'de> for GoldenVisitor {
            type Value = GoldenValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON value with unique object fields")
            }

            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(GoldenValue(Value::Null))
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> std::result::Result<Self::Value, E> {
                Ok(GoldenValue(value.into()))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(GoldenValue(value.into()))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(GoldenValue(value.into()))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<Self::Value, E> {
                Number::from_f64(value)
                    .map(|number| GoldenValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("invalid number in golden expectation"))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
                Ok(GoldenValue(value.into()))
            }

            fn visit_string<E: de::Error>(
                self,
                value: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(GoldenValue(value.into()))
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut entries: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = entries.next_element::<GoldenValue>()? {
                    values.push(value.0);
                }
                Ok(GoldenValue(Value::Array(values)))
            }

            fn visit_map<M: MapAccess<'de>>(
                self,
                mut entries: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut values = Map::new();
                while let Some(key) = entries.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate golden field {key}")));
                    }
                    values.insert(key, entries.next_value::<GoldenValue>()?.0);
                }
                Ok(GoldenValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(GoldenVisitor)
    }
}
