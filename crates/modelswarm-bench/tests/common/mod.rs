//! Test-only helpers: loads the frozen schemas from `experiments/schemas/`
//! and structurally validates JSON documents against the subset of JSON
//! Schema those files use (type/const/enum/pattern/minimum/maximum/
//! minLength/minItems/required/properties/additionalProperties/items/
//! propertyNames/format-date-time). No external validator dependency.

use std::path::PathBuf;

use serde_json::Value;

/// Repo-root `experiments/schemas/` directory, resolved from this crate.
pub fn schemas_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../experiments/schemas")
}

/// Loads a schema by file name.
pub fn load_schema(name: &str) -> Value {
    let path = schemas_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()))
}

/// Validates `instance` against `schema`; returns all violations found.
pub fn validate(schema: &Value, instance: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    validate_at(schema, instance, "$", &mut errors);
    errors
}

fn validate_at(schema: &Value, instance: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(spec) = schema.as_object() else {
        return; // boolean/absent subschema: nothing to check
    };

    if let Some(expected) = spec.get("const") {
        if instance != expected {
            errors.push(format!(
                "{path}: const mismatch, expected {expected}, got {instance}"
            ));
        }
    }
    if let Some(allowed) = spec.get("enum").and_then(Value::as_array) {
        if !allowed.contains(instance) {
            errors.push(format!(
                "{path}: {instance} not in enum {:?}",
                allowed.iter().map(|v| v.to_string()).collect::<Vec<_>>()
            ));
        }
    }
    if let Some(pattern) = spec.get("pattern").and_then(Value::as_str) {
        if let Value::String(text) = instance {
            if !pattern_matches(pattern, text) {
                errors.push(format!("{path}: {text:?} does not match {pattern:?}"));
            }
        } else {
            errors.push(format!(
                "{path}: pattern applies only to strings, got {instance}"
            ));
        }
    }
    if let Some(kind) = spec.get("type").and_then(Value::as_str) {
        check_type(kind, instance, path, errors);
    }
    if let Some(minimum) = spec.get("minimum").and_then(Value::as_f64) {
        if let Some(number) = instance.as_f64() {
            if number < minimum {
                errors.push(format!("{path}: {number} < minimum {minimum}"));
            }
        }
    }
    if let Some(maximum) = spec.get("maximum").and_then(Value::as_f64) {
        if let Some(number) = instance.as_f64() {
            if number > maximum {
                errors.push(format!("{path}: {number} > maximum {maximum}"));
            }
        }
    }
    if let Some(min_length) = spec.get("minLength").and_then(Value::as_u64) {
        if let Some(text) = instance.as_str() {
            if text.len() as u64 >= min_length {
                // minLength is a lower bound: fine.
            }
            if (text.len() as u64) < min_length {
                errors.push(format!(
                    "{path}: {:?} shorter than minLength {min_length}",
                    text
                ));
            }
        }
    }
    if let Some(min_items) = spec.get("minItems").and_then(Value::as_u64) {
        if let Some(items) = instance.as_array() {
            if (items.len() as u64) < min_items {
                errors.push(format!(
                    "{path}: {} items < minItems {min_items}",
                    items.len()
                ));
            }
        }
    }
    if let Some(format) = spec.get("format").and_then(Value::as_str) {
        if format == "date-time" {
            if let Some(text) = instance.as_str() {
                if !looks_like_rfc3339(text) {
                    errors.push(format!("{path}: {text:?} is not an RFC 3339 date-time"));
                }
            }
        }
    }

    match instance {
        Value::Object(map) => {
            if let Some(required) = spec.get("required").and_then(Value::as_array) {
                for key in required {
                    if let Some(key) = key.as_str() {
                        if !map.contains_key(key) {
                            errors.push(format!("{path}: missing required property {key:?}"));
                        }
                    }
                }
            }
            if let Some(properties) = spec.get("properties").and_then(Value::as_object) {
                for (key, subschema) in properties {
                    if let Some(value) = map.get(key) {
                        validate_at(subschema, value, &format!("{path}.{key}"), errors);
                    }
                }
            }
            if let Some(additional) = spec.get("additionalProperties") {
                let declared: Vec<String> = spec
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|props| props.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                match additional {
                    Value::Bool(false) => {
                        for key in map.keys() {
                            if !declared.contains(key) {
                                errors.push(format!(
                                    "{path}: additional property {key:?} not allowed"
                                ));
                            }
                        }
                    }
                    other if other.is_object() => {
                        let extra_schema = Value::Object(other.as_object().unwrap().clone());
                        for (key, value) in map {
                            if !declared.contains(key) {
                                validate_at(&extra_schema, value, &format!("{path}.{key}"), errors);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(property_names) = spec.get("propertyNames") {
                for key in map.keys() {
                    validate_at(property_names, &Value::String(key.clone()), path, errors);
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = spec.get("items") {
                for (index, item) in items.iter().enumerate() {
                    validate_at(item_schema, item, &format!("{path}[{index}]"), errors);
                }
            }
        }
        _ => {}
    }
}

fn check_type(kind: &str, instance: &Value, path: &str, errors: &mut Vec<String>) {
    let ok = match kind {
        "object" => instance.is_object(),
        "array" => instance.is_array(),
        "string" => instance.is_string(),
        "boolean" => instance.is_boolean(),
        "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
        "number" => instance.is_number(),
        _ => true,
    };
    if !ok {
        errors.push(format!("{path}: expected type {kind}, got {instance}"));
    }
}

fn looks_like_rfc3339(text: &str) -> bool {
    // YYYY-MM-DDTHH:MM:SSZ (the harness emits exactly this shape).
    let bytes = text.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && (bytes[10] == b'T' || bytes[10] == b't')
        && bytes[13] == b':'
        && bytes[16] == b':'
        && (text.ends_with('Z') || text.ends_with('z'))
        && text[..19]
            .chars()
            .take(19)
            .filter(|c| c.is_ascii_digit())
            .count()
            >= 14
}

/// Tiny pattern matcher for the shapes the frozen schemas use:
/// `^`, `$`, literal chars, `[0-9a-f]`-style classes, and `{n}` repetition
/// of the previous unit. Anything else is rejected loudly so an unexpected
/// schema pattern cannot silently pass.
fn pattern_matches(pattern: &str, text: &str) -> bool {
    #[derive(Debug, Clone)]
    enum Unit {
        Literal(char),
        Class(Vec<char>),
    }
    let body = pattern
        .strip_prefix('^')
        .and_then(|p| p.strip_suffix('$'))
        .unwrap_or_else(|| panic!("unsupported pattern shape: {pattern:?}"));

    let mut units: Vec<(Unit, u32)> = Vec::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        let unit = match c {
            '[' => {
                let mut class = Vec::new();
                let mut inner = chars.next().expect("unterminated class");
                while inner != ']' {
                    if inner == '-' && !class.is_empty() {
                        let range_start = class.pop().expect("class range start");
                        let range_end = chars.next().expect("class range end");
                        chars.next_if(|c| *c == '-');
                        let mut expanded = range_start as u32..=range_end as u32;
                        for code in expanded.by_ref() {
                            class.push(char::from_u32(code).expect("class range char"));
                        }
                    } else {
                        class.push(inner);
                    }
                    inner = chars.next().expect("unterminated class");
                }
                Unit::Class(class)
            }
            '\\' => Unit::Literal(chars.next().expect("dangling escape")),
            literal => Unit::Literal(literal),
        };
        let count = if chars.peek() == Some(&'{') {
            chars.next();
            let mut digits = String::new();
            while let Some(digit) = chars.next_if(|c| c.is_ascii_digit()) {
                digits.push(digit);
            }
            assert!(
                chars.next() == Some('}'),
                "unsupported repetition in {pattern:?}"
            );
            digits.parse().expect("repetition count")
        } else {
            1
        };
        units.push((unit, count));
    }

    // Match with simple backtracking over fixed-count units.
    fn match_units(units: &[(Unit, u32)], chars: &[char]) -> bool {
        let Some(((unit, count), rest)) = units.split_first() else {
            return chars.is_empty();
        };
        if chars.len() < *count as usize {
            return false;
        }
        for c in &chars[..*count as usize] {
            let ok = match unit {
                Unit::Literal(expected) => c == expected,
                Unit::Class(class) => class.contains(c),
            };
            if !ok {
                return false;
            }
        }
        match_units(rest, &chars[*count as usize..])
    }

    match_units(&units, &text.chars().collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pattern_matcher_handles_the_frozen_patterns() {
        assert!(pattern_matches(
            "^run-[0-9a-f]{16}$",
            "run-0123456789abcdef"
        ));
        assert!(!pattern_matches(
            "^run-[0-9a-f]{16}$",
            "run-0123456789ABCDEF"
        ));
        assert!(!pattern_matches("^run-[0-9a-f]{16}$", "run-short"));
        assert!(pattern_matches(
            "^msp1:[0-9a-f]{64}$",
            &format!("msp1:{}", "a".repeat(64))
        ));
        assert!(!pattern_matches("^msp1:[0-9a-f]{64}$", "msp1:xyz"));
        assert!(pattern_matches("^[0-9a-f]{64}$", &"0f".repeat(32)));
    }

    #[test]
    fn validator_catches_extra_properties_and_bad_types() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["a"],
            "properties": { "a": { "type": "integer", "minimum": 1 } }
        });
        assert!(validate(&schema, &json!({ "a": 3 })).is_empty());
        let errors = validate(&schema, &json!({ "a": 0, "b": 1 }));
        assert_eq!(errors.len(), 2, "{errors:?}");
    }
}
