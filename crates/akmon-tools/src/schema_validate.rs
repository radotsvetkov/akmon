//! JSON Schema validation for LLM tool arguments before dispatch.

use jsonschema::Validator;
use serde_json::Value as JsonValue;

/// Validates `args` against a tool's `parameters_schema`.
///
/// Empty object schemas (`{}`) skip validation (no constraints declared).
pub fn validate_tool_arguments(schema: &JsonValue, args: &JsonValue) -> Result<(), String> {
    if schema.as_object().is_some_and(|m| m.is_empty()) {
        return Ok(());
    }
    let validator = Validator::new(schema).map_err(|e| format!("invalid tool schema: {e}"))?;
    let errors: Vec<String> = validator.iter_errors(args).map(|e| e.to_string()).collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_schema_skips_validation() {
        assert!(validate_tool_arguments(&json!({}), &json!({"anything": 1})).is_ok());
    }

    #[test]
    fn required_field_enforced() {
        let schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        assert!(validate_tool_arguments(&schema, &json!({})).is_err());
        assert!(validate_tool_arguments(&schema, &json!({"path": "a.rs"})).is_ok());
    }
}
