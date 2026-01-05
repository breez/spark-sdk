/// Z-base-32 encoding alphabet
/// This alphabet is designed to be human-friendly and avoid ambiguous characters
const ZBASE32_ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";

/// Encode bytes as z-base-32 string
///
/// Z-base-32 is a base32 encoding variant optimized for human readability.
/// It encodes 5 bits per character and pads to full byte boundaries.
#[allow(clippy::arithmetic_side_effects)]
pub fn encode_zbase32(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }

    let mut result = Vec::with_capacity((data.len() * 8).div_ceil(5));
    let mut buffer: u16 = 0;
    let mut bits_in_buffer: u8 = 0;

    for &byte in data {
        buffer = (buffer << 8) | u16::from(byte);
        bits_in_buffer += 8;

        while bits_in_buffer >= 5 {
            bits_in_buffer -= 5;
            let index = ((buffer >> bits_in_buffer) & 0x1F) as usize;
            result.push(ZBASE32_ALPHABET[index]);
        }
    }

    // Handle remaining bits
    if bits_in_buffer > 0 {
        let index = ((buffer << (5 - bits_in_buffer)) & 0x1F) as usize;
        result.push(ZBASE32_ALPHABET[index]);
    }

    String::from_utf8(result).expect("zbase32 alphabet is valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_zbase32() {
        // Test vectors verified against z-base-32 specification
        assert_eq!(encode_zbase32(b""), "");
        assert_eq!(encode_zbase32(b"\x00"), "yy");
        // Well-known test vector: "hello" -> "pb1sa5dx"
        assert_eq!(encode_zbase32(b"hello"), "pb1sa5dx");

        // Additional verified test cases
        assert_eq!(encode_zbase32(b"f"), "ca");
        assert_eq!(encode_zbase32(b"fo"), "c3zo");
        assert_eq!(encode_zbase32(b"foo"), "c3zs6");

        // Test with signature-length data (65 bytes typical for ECDSA recoverable)
        let test_sig = vec![0xAB; 65];
        let encoded = encode_zbase32(&test_sig);
        assert!(!encoded.is_empty());
        assert_eq!(encoded.len(), 104); // 65 bytes * 8 bits / 5 bits per char = 104 chars
        assert!(
            encoded
                .chars()
                .all(|c| ZBASE32_ALPHABET.contains(&(c as u8)))
        );
    }

    #[test]
    fn test_encode_zbase32_alphabet_coverage() {
        // Test that our implementation produces valid zbase32 characters
        let test_data = vec![0u8, 255, 128, 64, 32, 16, 8, 4, 2, 1];
        let encoded = encode_zbase32(&test_data);
        for ch in encoded.chars() {
            assert!(
                ZBASE32_ALPHABET.contains(&(ch as u8)),
                "Invalid character: {ch}"
            );
        }
    }

    #[test]
    fn test_encode_zbase32_original_test_vectors() {
        // Test vectors from the original zbase32 crate
        // https://gitlab.com/pgerber/zbase32-rust/-/blob/master/src/lib.rs

        // Full byte encodings (bits % 8 == 0)
        assert_eq!(encode_zbase32(&[0x00]), "yy");
        assert_eq!(encode_zbase32(&[0xf0, 0xbf, 0xc7]), "6n9hq");
        assert_eq!(encode_zbase32(&[0xd4, 0x7a, 0x04]), "4t7ye");
        assert_eq!(
            encode_zbase32(&[
                0x00, 0x44, 0x32, 0x14, 0xc7, 0x42, 0x54, 0xb6, 0x35, 0xcf, 0x84, 0x65, 0x3a, 0x56,
                0xd7, 0xc6, 0x75, 0xbe, 0x77, 0xdf
            ]),
            "ybndrfg8ejkmcpqxot1uwisza345h769"
        );
    }

    #[test]
    fn test_encode_zbase32_known_strings() {
        // Additional test cases for common string encodings
        // Verified against reference implementation
        assert_eq!(encode_zbase32(b"f"), "ca");
        assert_eq!(encode_zbase32(b"fo"), "c3zo");
        assert_eq!(encode_zbase32(b"foo"), "c3zs6");
        assert_eq!(encode_zbase32(b"foob"), "c3zs6ao");
        assert_eq!(encode_zbase32(b"fooba"), "c3zs6aub");
        assert_eq!(encode_zbase32(b"foobar"), "c3zs6aubqe");
    }
}
