//! Casing- and tag-tolerant JSON assertions.
//!
//! The Rust CLI prints core models as `snake_case` JSON with externally
//! tagged enums (`{"Bolt11Invoice": {...}}`); the wasm port prints camelCase
//! JSON with `{"type": "bolt11Invoice", ...}` tags and `BigInt` values as
//! strings.
//! Shared scenarios must assert on both, so path segments and values are
//! compared after normalization.

use anyhow::{Result, bail};
use serde_json::Value;

/// Strip echoed REPL prompts from line starts. Some ports (`JLine`'s dumb
/// terminal) write the prompt with no trailing newline, so a result document
/// opens as `breez-spark-cli [regtest]> {` instead of at column 0.
pub fn strip_prompts(chunk: &str) -> String {
    let mut out = String::with_capacity(chunk.len());
    for line in chunk.lines() {
        let mut line = line;
        while let Some(rest) = line
            .strip_prefix("breez-spark-cli [regtest]> ")
            .or_else(|| line.strip_prefix("breez-spark-cli [mainnet]> "))
        {
            line = rest;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Extract the JSON documents a transcript chunk contains. A document starts
/// at a line whose first column is `{` or `[` and ends when the accumulated
/// lines parse; interleaved log lines never start with a brace.
pub fn extract_json_docs(chunk: &str) -> Vec<Value> {
    let mut docs = Vec::new();
    let mut acc: Option<String> = None;
    for line in chunk.lines() {
        if let Some(buf) = &mut acc {
            buf.push_str(line);
            if let Ok(value) = serde_json::from_str::<Value>(buf) {
                docs.push(value);
                acc = None;
            } else {
                buf.push('\n');
            }
        } else if line.starts_with('{') || line.starts_with('[') {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                docs.push(value);
            } else {
                let mut buf = line.to_string();
                buf.push('\n');
                acc = Some(buf);
            }
        }
    }
    docs
}

/// Lowercase and strip underscores so `payment_request`, `paymentRequest`,
/// and `PaymentRequest` all compare equal.
fn normalize(s: &str) -> String {
    s.to_lowercase().replace('_', "")
}

/// Resolve a dot-separated path against a JSON value. Object keys match
/// after normalization; numeric segments index arrays. An enum-tag segment
/// matches either an externally tagged wrapper key (descends into its value)
/// or a `type` field on the current object (stays in place). The empty path
/// addresses the document itself.
pub fn lookup_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    let mut current = root;
    for segment in path.split('.') {
        let wanted = normalize(segment);
        match current {
            Value::Object(map) => {
                if let Some((_, value)) = map.iter().find(|(key, _)| normalize(key) == wanted) {
                    current = value;
                } else if map
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|tag| normalize(tag) == wanted)
                {
                    // Tag-style enum: the segment names the variant of the
                    // object itself; the fields live alongside the tag.
                } else {
                    return None;
                }
            }
            Value::Array(items) => {
                let index: usize = segment.parse().ok()?;
                current = items.get(index)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Render a JSON value the way a scenario writes it: strings bare, others as
/// their JSON text.
pub fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Check one `expect_json` matcher against the value found at its path.
///
/// Matcher forms: a bare value (tolerant equality), `{"gte": n}` (numeric
/// floor, accepts numeric strings), `{"exists": true|false}`.
pub fn check_matcher(matcher: &Value, found: Option<&Value>) -> Result<()> {
    if let Value::Object(map) = matcher {
        if let Some(expected) = map.get("exists").and_then(Value::as_bool) {
            let exists = found.is_some_and(|v| !v.is_null());
            if exists != expected {
                bail!("expected exists={expected}, value was {found:?}");
            }
            return Ok(());
        }
        if let Some(floor) = map.get("gte") {
            let Some(floor) = as_number(floor) else {
                bail!("gte bound is not numeric: {floor}");
            };
            let Some(actual) = found.and_then(as_number) else {
                bail!("expected a number >= {floor}, value was {found:?}");
            };
            if actual < floor {
                bail!("expected >= {floor}, got {actual}");
            }
            return Ok(());
        }
    }

    let Some(found) = found else {
        bail!("path not found in output");
    };
    let expected = value_to_string(matcher).to_lowercase();
    let actual = value_to_string(found).to_lowercase();
    if expected != actual {
        bail!("expected '{expected}', got '{actual}'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn strips_echoed_prompts_from_line_starts() {
        let chunk = "breez-spark-cli [regtest]> {\n  \"a\": 1\n}\nplain line\n";
        let stripped = strip_prompts(chunk);
        assert_eq!(extract_json_docs(&stripped), vec![json!({"a": 1})]);
        assert!(stripped.contains("plain line"));
    }

    #[test]
    fn extracts_pretty_and_inline_docs_and_skips_noise() {
        let chunk =
            "Breez SDK: noise\n{\n  \"a\": 1\n}\nError: nope\n{\"b\": 2}\nEvent: {\"c\": 3}\n";
        let docs = extract_json_docs(chunk);
        assert_eq!(docs, vec![json!({"a": 1}), json!({"b": 2})]);
    }

    #[test]
    fn path_matches_across_casings() {
        let rust = json!({"payment_request": {"amount_msat": 5}});
        let wasm = json!({"paymentRequest": {"amountMsat": 5}});
        for doc in [rust, wasm] {
            assert_eq!(
                lookup_path(&doc, "payment_request.amount_msat"),
                Some(&json!(5))
            );
        }
    }

    #[test]
    fn path_bridges_enum_tags() {
        let rust = json!({"Bolt11Invoice": {"amount_msat": 7}});
        let wasm = json!({"type": "bolt11Invoice", "amountMsat": 7});
        for doc in [rust, wasm] {
            assert_eq!(
                lookup_path(&doc, "bolt11_invoice.amount_msat"),
                Some(&json!(7)),
            );
        }
    }

    #[test]
    fn empty_path_addresses_the_document() {
        let doc = json!({});
        assert_eq!(lookup_path(&doc, ""), Some(&doc));
        check_matcher(&json!({}), lookup_path(&doc, "")).unwrap();
    }

    #[test]
    fn path_indexes_arrays() {
        let doc = json!({"payments": [{"id": "x"}, {"id": "y"}]});
        assert_eq!(lookup_path(&doc, "payments.1.id"), Some(&json!("y")));
        assert_eq!(lookup_path(&doc, "payments.2.id"), None);
    }

    #[test]
    fn equality_tolerates_case_and_bigint_strings() {
        check_matcher(&json!("completed"), Some(&json!("Completed"))).unwrap();
        check_matcher(&json!("1000"), Some(&json!(1000))).unwrap();
        check_matcher(&json!(1000), Some(&json!("1000"))).unwrap();
        assert!(check_matcher(&json!("completed"), Some(&json!("failed"))).is_err());
        assert!(check_matcher(&json!("completed"), None).is_err());
    }

    #[test]
    fn gte_accepts_numbers_and_numeric_strings() {
        check_matcher(&json!({"gte": 10}), Some(&json!(11))).unwrap();
        check_matcher(&json!({"gte": 10}), Some(&json!("10"))).unwrap();
        assert!(check_matcher(&json!({"gte": 10}), Some(&json!(9))).is_err());
        assert!(check_matcher(&json!({"gte": 10}), Some(&json!("abc"))).is_err());
        assert!(check_matcher(&json!({"gte": 10}), None).is_err());
    }

    #[test]
    fn exists_checks_presence_and_null() {
        check_matcher(&json!({"exists": true}), Some(&json!("x"))).unwrap();
        check_matcher(&json!({"exists": false}), None).unwrap();
        check_matcher(&json!({"exists": false}), Some(&json!(null))).unwrap();
        assert!(check_matcher(&json!({"exists": true}), Some(&json!(null))).is_err());
        assert!(check_matcher(&json!({"exists": true}), None).is_err());
    }
}
