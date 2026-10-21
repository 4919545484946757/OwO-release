//! Reject duplicate decoded object keys at every depth before DTO decoding.
use serde::{
    de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use std::{collections::HashSet, fmt};

struct Unique;
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Unique, A::Error> {
                while seq.next_element::<Unique>()?.is_some() {}
                Ok(Unique)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Unique, A::Error> {
                let mut keys = HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key) {
                        return Err(de::Error::custom("duplicate JSON object key"));
                    }
                    map.next_value::<Unique>()?;
                }
                Ok(Unique)
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub(crate) fn from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    // Serde JSON's default recursion limit remains active in both passes.
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    Unique::deserialize(&mut deserializer)?;
    deserializer.end()?;
    serde_json::from_slice(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicates_at_any_depth_and_escaped_aliases_are_rejected() {
        for bytes in [
            br#"{"action":1,"action":2}"#.as_slice(),
            br#"{"context":[{"text":1,"text":2}]}"#.as_slice(),
            br#"{"a":1,"\u0061":2}"#.as_slice(),
            br#"{"x":{"opaque":{"grant":true,"grant":false}}}"#.as_slice(),
        ] {
            // Generic serde_json::Value normally silently keeps the last key.
            assert!(serde_json::from_slice::<serde_json::Value>(bytes).is_ok());
            let error = from_slice::<serde_json::Value>(bytes).unwrap_err();
            assert!(error.to_string().contains("duplicate JSON object key"));
            assert!(!error.to_string().contains("grant"));
        }
    }
    #[test]
    fn valid_objects_keep_semantics_and_independent_objects_can_reuse_keys() {
        let bytes = br#"{"x":[{"a":1},{"a":2}],"s":"unicode: \u4f60","n":null,"b":false,"f":1.5}"#;
        assert_eq!(
            from_slice::<serde_json::Value>(bytes).unwrap(),
            serde_json::from_slice::<serde_json::Value>(bytes).unwrap()
        );
    }
    #[test]
    fn trailing_values_malformed_input_and_deep_nesting_fail() {
        for bytes in [b"{} {}".as_slice(), b"{\"x\":}".as_slice(), b"".as_slice()] {
            assert!(from_slice::<serde_json::Value>(bytes).is_err());
        }
        let deep = format!("{}0{}", "[".repeat(200), "]".repeat(200));
        assert!(from_slice::<serde_json::Value>(deep.as_bytes()).is_err());
    }
}
