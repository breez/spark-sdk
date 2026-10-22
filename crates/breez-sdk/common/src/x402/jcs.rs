//! JSON Canonicalization Scheme (RFC 8785) over `serde_json::Value`.

use std::fmt::{self, Write};

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// 2^53 - 1. Integers beyond it have no exact IEEE-754 double, so two distinct
/// literals could share a canonical form. They are rejected rather than rounded.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct JcsError(pub String);

impl fmt::Display for JcsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Parses JSON text, rejecting duplicate object member names, which RFC 8785
/// places outside its data model. `serde_json` alone keeps the last duplicate.
pub(crate) fn parse_strict(text: &str) -> Result<Value, JcsError> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value = StrictValue::deserialize(&mut deserializer)
        .map_err(|e| JcsError(format!("invalid JSON: {e}")))?
        .0;
    deserializer
        .end()
        .map_err(|e| JcsError(format!("invalid JSON: {e}")))?;
    Ok(value)
}

/// Serializes a value as RFC 8785 canonical JSON.
pub(crate) fn canonicalize(value: &Value) -> Result<String, JcsError> {
    let mut out = String::new();
    write_value(&mut out, value)?;
    Ok(out)
}

fn write_value(out: &mut String, value: &Value) -> Result<(), JcsError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&format_number(n)?),
        Value::String(s) => write_string(out, s)?,
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            // RFC 8785 orders members by UTF-16 code units, which differs from
            // the UTF-8 byte order of the map for characters outside the BMP.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, key)?;
                out.push(':');
                write_value(out, &map[key])?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// `serde_json` escapes exactly what RFC 8785 requires: `"`, `\` and controls
/// below U+0020, using the short forms and lowercase `\u00xx` otherwise.
fn write_string(out: &mut String, s: &str) -> Result<(), JcsError> {
    let escaped = serde_json::to_string(s).map_err(|e| JcsError(e.to_string()))?;
    out.push_str(&escaped);
    Ok(())
}

fn format_number(n: &Number) -> Result<String, JcsError> {
    if let Some(u) = n.as_u64() {
        if u > MAX_SAFE_INTEGER {
            return Err(JcsError(format!("integer {u} is outside the safe range")));
        }
        return Ok(u.to_string());
    }
    if let Some(i) = n.as_i64() {
        if i.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(JcsError(format!("integer {i} is outside the safe range")));
        }
        return Ok(i.to_string());
    }
    let f = n
        .as_f64()
        .ok_or_else(|| JcsError(format!("unsupported number {n}")))?;
    #[allow(clippy::cast_precision_loss)]
    if f.fract() == 0.0 && f.abs() > MAX_SAFE_INTEGER as f64 {
        return Err(JcsError(format!("integer {n} is outside the safe range")));
    }
    Ok(format_es_number(f))
}

/// Formats a finite double like ECMAScript `Number.prototype.toString`
/// (ECMA-262 7.1.12.1). Rust's exponential formatting already yields the
/// shortest round-tripping digits, so only the layout rules are applied here.
fn format_es_number(f: f64) -> String {
    if f == 0.0 {
        return "0".to_string();
    }
    if f < 0.0 {
        return format!("-{}", format_es_number(-f));
    }
    let exp_form = format!("{f:e}");
    let (mantissa, exponent) = exp_form
        .split_once('e')
        .expect("exponential format has an exponent");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i32 = exponent.parse().expect("exponent is an integer");
    let k = i32::try_from(digits.len()).expect("digit count fits in i32");
    let n = exponent.saturating_add(1);

    let mut out = String::new();
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n(
            '0',
            usize::try_from(n.saturating_sub(k)).unwrap_or(0),
        ));
    } else if 0 < n && n <= 21 {
        let split = usize::try_from(n).unwrap_or(0);
        out.push_str(&digits[..split]);
        out.push('.');
        out.push_str(&digits[split..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n(
            '0',
            usize::try_from(n.saturating_neg()).unwrap_or(0),
        ));
        out.push_str(&digits);
    } else {
        let e = n.saturating_sub(1);
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let _ = write!(
            out,
            "e{}{}",
            if e < 0 { '-' } else { '+' },
            e.unsigned_abs()
        );
    }
    out
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor).map(StrictValue)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite number"))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_string()))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(StrictValue(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut out = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if out.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate member {key:?}")));
            }
            let StrictValue(value) = map.next_value()?;
            out.insert(key, value);
        }
        Ok(Value::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[macros::test_all]
    fn formats_numbers_like_ecmascript() {
        let cases = [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (1.5, "1.5"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123_456_789_012_345_680_000.0, "123456789012345680000"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (5e-324, "5e-324"),
            (1.797_693_134_862_315_7e308, "1.7976931348623157e+308"),
            (333_333_333.333_333_3, "333333333.3333333"),
        ];
        for (input, expected) in cases {
            assert_eq!(format_es_number(input), expected, "formatting {input:e}");
        }
    }

    #[macros::test_all]
    fn sorts_members_by_utf16() {
        // U+1F600 encodes as a surrogate pair (0xD83D...), which sorts before
        // U+FB33 (0xFB33) in UTF-16 even though it sorts after it in UTF-8.
        let value = parse_strict("{\"\\ufb33\":1,\"\\ud83d\\ude00\":2,\"a\":3}").unwrap();
        assert_eq!(
            canonicalize(&value).unwrap(),
            "{\"a\":3,\"\u{1F600}\":2,\"\u{FB33}\":1}"
        );
    }

    #[macros::test_all]
    fn escapes_controls_only() {
        let value = parse_strict(r#"["\u0001\n\t\"\\/\u007f é"]"#).unwrap();
        assert_eq!(
            canonicalize(&value).unwrap(),
            "[\"\\u0001\\n\\t\\\"\\\\/\u{7f} é\"]"
        );
    }

    #[macros::test_all]
    fn rejects_outside_data_model() {
        assert!(parse_strict(r#"{"a":1,"a":2}"#).is_err());
        assert!(parse_strict(r#""\ud800""#).is_err());
        assert!(canonicalize(&parse_strict("9007199254740993").unwrap()).is_err());
        assert!(canonicalize(&parse_strict("1e300").unwrap()).is_err());
        assert_eq!(
            canonicalize(&parse_strict("9007199254740991").unwrap()).unwrap(),
            "9007199254740991"
        );
    }
}
