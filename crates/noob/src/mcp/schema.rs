//! Client-side validation of `mcp_call` args against a tool's cached JSON
//! Schema, plus the compact parameter sketch shown in catalogs. Deliberately
//! shallow: required keys and primitive types at the top level, permissive
//! about everything it does not understand. A validation miss costs one
//! local round trip with the expected schema attached, instead of a wire
//! error the model cannot see the shape of.

use serde_json::Value;

/// Validate `args` against `schema`. Returns every problem found, joined,
/// so the model can fix the whole call in one retry.
pub fn validate(schema: &Value, args: &Value) -> Result<(), String> {
    let Some(schema) = schema.as_object() else {
        return Ok(()); // no usable schema: permissive
    };
    let Some(args_map) = args.as_object() else {
        return Err("args must be a JSON object".to_string());
    };
    let mut problems = Vec::new();
    let properties = schema.get("properties").and_then(Value::as_object);

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !args_map.contains_key(key) {
                problems.push(format!("missing required {key:?}"));
            }
        }
    }
    if let Some(properties) = properties {
        for (key, value) in args_map {
            let Some(prop) = properties.get(key) else {
                if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                    problems.push(format!("unknown key {key:?}"));
                }
                continue;
            };
            let Some(expected) = prop.get("type").and_then(Value::as_str) else {
                continue; // unions, $refs, anyOf: out of scope, permissive
            };
            if !type_matches(expected, value) {
                problems.push(format!(
                    "{key:?} must be a {expected}, got {}",
                    type_name(value)
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true, // an exotic type keyword: permissive
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// A one-line parameter sketch for the connect catalog:
/// `(query: string, limit?: integer)`. Required params first (schema order,
/// which serde keeps sorted), optionals marked with `?`.
pub fn sketch(schema: &Value) -> String {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return "()".to_string();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    for (key, prop) in properties {
        let ty = prop.get("type").and_then(Value::as_str).unwrap_or("any");
        let opt = if required.contains(&key.as_str()) {
            ""
        } else {
            "?"
        };
        parts.push(format!("{key}{opt}: {ty}"));
    }
    // Required first keeps the important arguments visible when a catalog
    // line gets long.
    parts.sort_by_key(|p| p.contains('?'));
    format!("({})", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn echo_schema() -> Value {
        json!({"type": "object",
            "properties": {
                "text": {"type": "string"},
                "count": {"type": "integer"},
                "deep": {"type": "object"}
            },
            "required": ["text"]})
    }

    #[test]
    fn accepts_valid_args_and_extra_keys_by_default() {
        assert!(validate(&echo_schema(), &json!({"text": "hi"})).is_ok());
        assert!(validate(&echo_schema(), &json!({"text": "hi", "count": 3})).is_ok());
        // additionalProperties unset: unknown keys pass.
        assert!(validate(&echo_schema(), &json!({"text": "hi", "extra": 1})).is_ok());
    }

    #[test]
    fn reports_every_problem_at_once() {
        let err = validate(&echo_schema(), &json!({"count": "three"})).unwrap_err();
        assert!(err.contains("missing required \"text\""), "{err}");
        assert!(
            err.contains("\"count\" must be a integer, got string"),
            "{err}"
        );
    }

    #[test]
    fn additional_properties_false_rejects_unknown_keys() {
        let mut schema = echo_schema();
        schema["additionalProperties"] = json!(false);
        let err = validate(&schema, &json!({"text": "x", "bogus": 1})).unwrap_err();
        assert!(err.contains("unknown key \"bogus\""), "{err}");
    }

    #[test]
    fn integer_vs_number_distinction() {
        let schema = json!({"type": "object", "properties": {
            "i": {"type": "integer"}, "n": {"type": "number"}}});
        assert!(validate(&schema, &json!({"i": 3, "n": 3.5})).is_ok());
        assert!(
            validate(&schema, &json!({"n": 3})).is_ok(),
            "an int is a number"
        );
        let err = validate(&schema, &json!({"i": 3.5})).unwrap_err();
        assert!(err.contains("must be a integer"), "{err}");
    }

    #[test]
    fn schemas_we_do_not_understand_are_permissive() {
        assert!(validate(&json!(null), &json!({"anything": 1})).is_ok());
        let union = json!({"type": "object", "properties": {
            "x": {"anyOf": [{"type": "string"}, {"type": "integer"}]}}});
        assert!(validate(&union, &json!({"x": []})).is_ok());
    }

    #[test]
    fn sketch_marks_optionals_and_orders_required_first() {
        assert_eq!(
            sketch(&echo_schema()),
            "(text: string, count?: integer, deep?: object)"
        );
        assert_eq!(sketch(&json!({"type": "object"})), "()");
        assert_eq!(
            sketch(&json!({"properties": {"q": {}}, "required": ["q"]})),
            "(q: any)"
        );
    }
}
