// Simplified percent encoding/decoding based on RFC 3986
// Adapted from percent-encoding-rfc3986 crate

/// Percent-encode a string according to RFC 3986.
/// Encodes all non-alphanumeric characters except: - _ . ~
pub fn encode(input: &str) -> String {
    let mut result = String::new();

    for byte in input.bytes() {
        match byte {
            // Unreserved characters (ALPHA / DIGIT / "-" / "." / "_" / "~")
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                result.push(byte as char);
            }
            // All other characters must be percent-encoded
            _ => {
                use std::fmt::Write;
                result.push('%');
                write!(result, "{byte:02X}").expect("writing to String cannot fail");
            }
        }
    }

    result
}

/// Percent-decode a string according to RFC 3986.
/// Returns an error if the encoding is invalid (e.g., '%' not followed by two hex digits).
pub fn decode(input: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut chars = input.chars();

    while let Some(c) = chars.next() {
        if c == '%' {
            // Must be followed by exactly two hex digits
            let hex1 = chars.next().ok_or_else(|| {
                "Invalid percent encoding: incomplete escape sequence".to_string()
            })?;
            let hex2 = chars.next().ok_or_else(|| {
                "Invalid percent encoding: incomplete escape sequence".to_string()
            })?;

            let hex_str: String = [hex1, hex2].iter().collect();
            let byte = u8::from_str_radix(&hex_str, 16).map_err(|_| {
                format!("Invalid percent encoding: invalid hex characters '{hex_str}'")
            })?;

            bytes.push(byte);
        } else {
            // Any non-'%' character is passed through as-is
            let mut buf = [0u8; 4];
            let encoded = c.encode_utf8(&mut buf);
            bytes.extend_from_slice(encoded.as_bytes());
        }
    }

    String::from_utf8(bytes)
        .map_err(|e| format!("Invalid percent encoding: invalid UTF-8 sequence: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_simple() {
        assert_eq!(encode("hello"), "hello");
        assert_eq!(encode("hello world"), "hello%20world");
        assert_eq!(encode(""), "");
    }

    #[test]
    fn test_encode_special_chars() {
        assert_eq!(encode("a+b"), "a%2Bb");
        assert_eq!(encode("test@example.com"), "test%40example.com");
        assert_eq!(encode("a=b&c=d"), "a%3Db%26c%3Dd");
    }

    #[test]
    fn test_encode_unreserved() {
        // RFC 3986 unreserved characters should not be encoded
        assert_eq!(encode("ABCabc123-_.~"), "ABCabc123-_.~");
    }

    #[test]
    fn test_encode_utf8() {
        assert_eq!(encode("café"), "caf%C3%A9");
        assert_eq!(encode("日本"), "%E6%97%A5%E6%9C%AC");
    }

    #[test]
    fn test_decode_simple() {
        assert_eq!(decode("hello").unwrap(), "hello");
        assert_eq!(decode("hello%20world").unwrap(), "hello world");
        assert_eq!(decode("").unwrap(), "");
    }

    #[test]
    fn test_decode_special_chars() {
        assert_eq!(decode("a%2Bb").unwrap(), "a+b");
        assert_eq!(decode("test%40example.com").unwrap(), "test@example.com");
        assert_eq!(decode("a%3Db%26c%3Dd").unwrap(), "a=b&c=d");
    }

    #[test]
    fn test_decode_utf8() {
        assert_eq!(decode("caf%C3%A9").unwrap(), "café");
        assert_eq!(decode("%E6%97%A5%E6%9C%AC").unwrap(), "日本");
    }

    #[test]
    fn test_decode_errors() {
        // Incomplete escape sequence
        assert!(decode("incomplete%2").is_err());
        assert!(decode("incomplete%").is_err());

        // Invalid hex characters
        assert!(decode("invalid%ZZ").is_err());
        assert!(decode("invalid%GG").is_err());
    }

    #[test]
    fn test_roundtrip() {
        let test_cases = vec![
            "hello world",
            "test@example.com",
            "a+b=c&d=e",
            "path/to/resource?query=value",
            "café ☕",
            "ABCabc123-_.~",
        ];

        for case in test_cases {
            let encoded = encode(case);
            let decoded = decode(&encoded).unwrap();
            assert_eq!(decoded, case, "Roundtrip failed for: {case}");
        }
    }

    #[test]
    fn test_decode_mixed() {
        // Mix of encoded and non-encoded characters
        assert_eq!(decode("hello%20world%21").unwrap(), "hello world!");
        assert_eq!(decode("test%2Bvalue").unwrap(), "test+value");
    }

    #[test]
    fn test_encode_all_ascii() {
        // Test that all ASCII non-unreserved characters get encoded
        let input = "!\"#$%&'()*+,/:;<=>?@[\\]^`{|}";
        let encoded = encode(input);
        // Should not contain any of the original special characters except as hex
        assert!(!encoded.contains('!'));
        assert!(!encoded.contains('"'));
        assert!(!encoded.contains('&'));
        // But should contain % for encoding
        assert!(encoded.contains('%'));
    }

    #[test]
    fn test_percent_sign_encoding() {
        // Critical: '%' must ALWAYS be encoded per RFC 3986
        assert_eq!(encode("%"), "%25");
        assert_eq!(encode("100%"), "100%25");
        assert_eq!(encode("%20"), "%2520");
    }

    #[test]
    fn test_percent_sign_roundtrip() {
        let cases = vec!["%", "100%", "test%value"];
        for case in cases {
            let encoded = encode(case);
            let decoded = decode(&encoded).unwrap();
            assert_eq!(decoded, case, "Roundtrip failed for: {case}");
        }
    }

    #[test]
    fn test_case_insensitive_hex() {
        // Hex digits can be uppercase or lowercase
        assert_eq!(decode("%2b").unwrap(), "+");
        assert_eq!(decode("%2B").unwrap(), "+");
        assert_eq!(decode("%c3%a9").unwrap(), "é");
        assert_eq!(decode("%C3%A9").unwrap(), "é");
    }

    #[test]
    fn test_decode_with_literal_non_ascii() {
        // Test decoding when input contains literal non-ASCII characters
        // (not percent-encoded, but should pass through)
        assert_eq!(decode("café").unwrap(), "café");
        assert_eq!(decode("hello-世界").unwrap(), "hello-世界");
    }
}
