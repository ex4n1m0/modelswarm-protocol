//! Canonical JSON for signed payloads (msp-v1 §2.2).
//!
//! Same rules as `modelswarm_types::canonical` / `modelswarm_identity::canonical`
//! (kept local so this crate has no dependency on them): recursively sorted
//! keys, compact form, integers decimal, JSON-minimum string escaping.
//! Signed lease payloads contain no floats; a float fails closed.

use serde_json::Value;

/// Renders `value` as canonical JSON, or `None` if it contains a
/// floating-point number (forbidden in signed payloads).
pub fn canonical_json(value: &Value) -> Option<String> {
    let mut out = String::new();
    write_canonical(value, &mut out).then_some(out)
}

fn write_canonical(value: &Value, out: &mut String) -> bool {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else {
                return false; // floats are never canonical-signed
            }
        }
        Value::String(s) => match serde_json::to_string(s) {
            Ok(encoded) => out.push_str(&encoded),
            Err(_) => return false,
        },
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !write_canonical(item, out) {
                    return false;
                }
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let encoded = match serde_json::to_string(key.as_str()) {
                    Ok(e) => e,
                    Err(_) => return false,
                };
                out.push_str(&encoded);
                out.push(':');
                if !write_canonical(&map[key.as_str()], out) {
                    return false;
                }
            }
            out.push('}');
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorts_keys_and_rejects_floats() {
        let v = json!({"peer_id": "p", "audit_epoch": 7, "can_host": true});
        assert_eq!(
            canonical_json(&v),
            Some(r#"{"audit_epoch":7,"can_host":true,"peer_id":"p"}"#.to_string())
        );
        assert_eq!(canonical_json(&json!({"slots": 0.5})), None);
    }
}
