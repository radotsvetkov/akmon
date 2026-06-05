//! Deterministic canonical JSON byte encoding for content-addressed objects.

use serde_json::{Map, Value};

/// Serializes a [`serde_json::Value`] to canonical bytes with object keys sorted
/// recursively.
///
/// Two JSON values that differ only in object key ordering produce identical
/// bytes, so content-addressed object hashes are stable regardless of how the
/// upstream trace happened to order its fields.
///
/// # Errors
///
/// Returns an error string when `serde_json` fails to serialize the canonicalized
/// value (for example, on a map with non-string keys, which cannot occur for
/// values built from JSON input).
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, String> {
    let canonical = canonicalize(value);
    serde_json::to_vec(&canonical).map_err(|err| err.to_string())
}

/// Recursively rewrites a value so every object's keys are in sorted order.
fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = Map::new();
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(child) = map.get(key) {
                    sorted.insert(key.clone(), canonicalize(child));
                }
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_independence() {
        let a = json!({"b": 1, "a": 2, "c": {"z": 1, "y": 2}});
        let b = json!({"c": {"y": 2, "z": 1}, "a": 2, "b": 1});
        let ba = canonical_json_bytes(&a).unwrap_or_else(|_| unreachable!());
        let bb = canonical_json_bytes(&b).unwrap_or_else(|_| unreachable!());
        assert_eq!(ba, bb);
    }

    #[test]
    fn nested_arrays_of_objects_are_canonicalized() {
        let a = json!({"items": [{"b": 1, "a": 2}, {"d": 4, "c": 3}]});
        let b = json!({"items": [{"a": 2, "b": 1}, {"c": 3, "d": 4}]});
        assert_eq!(
            canonical_json_bytes(&a).unwrap_or_else(|_| unreachable!()),
            canonical_json_bytes(&b).unwrap_or_else(|_| unreachable!())
        );
    }

    #[test]
    fn distinct_content_distinct_bytes() {
        let a = json!({"a": 1});
        let b = json!({"a": 2});
        assert_ne!(
            canonical_json_bytes(&a).unwrap_or_else(|_| unreachable!()),
            canonical_json_bytes(&b).unwrap_or_else(|_| unreachable!())
        );
    }
}
