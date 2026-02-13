use bitcoin::hashes::{Hash, HashEngine, sha256};
use std::collections::BTreeMap;

/// A hasher that implements BIP-340 tagged hashing with length-prefixed values.
///
/// The hash is computed using the BIP-340 tagged hash pattern:
/// - tagHash = SHA256(serializeTag(tag))
/// - result = SHA256(tagHash || tagHash || serialized values)
///
/// Each value is serialized as `[8-byte BE length][value bytes]`.
///
/// # Example
///
/// ```
/// use spark::utils::tagged_hasher::TaggedHasher;
///
/// let hash = TaggedHasher::new(&["spark", "deposit", "proof_of_possession"])
///     .add_bytes(&[1, 2, 3])
///     .add_bytes(b"hello")
///     .hash();
/// ```
pub struct TaggedHasher {
    tag_hash: [u8; 32],
    values: Vec<u8>,
}

impl TaggedHasher {
    /// Creates a new TaggedHasher with the given hierarchical domain tag.
    ///
    /// The tag is a hierarchical path, such as `["spark", "deposit", "proof_of_possession"]`.
    pub fn new(tag: &[&str]) -> Self {
        Self {
            tag_hash: Self::compute_tag_hash(tag),
            values: Vec::new(),
        }
    }

    /// Adds a byte slice to the hash computation.
    ///
    /// The bytes are serialized as `[8-byte BE length][value bytes]`.
    #[must_use]
    pub fn add_bytes(mut self, bytes: &[u8]) -> Self {
        self.write_length_prefixed(bytes);
        self
    }

    /// Adds a string to the hash computation.
    ///
    /// The string is encoded as UTF-8 bytes and serialized as `[8-byte BE length][UTF-8 bytes]`.
    #[must_use]
    pub fn add_string(mut self, s: &str) -> Self {
        self.write_length_prefixed(s.as_bytes());
        self
    }

    /// Adds a u64 value to the hash computation.
    ///
    /// The value is serialized as `[8-byte BE length (always 8)][8-byte BE value]`.
    #[must_use]
    pub fn add_u64(mut self, value: u64) -> Self {
        let value_bytes = value.to_be_bytes();
        self.write_length_prefixed(&value_bytes);
        self
    }

    /// Adds a map of string keys to byte values to the hash computation.
    ///
    /// The map is hashed in a deterministic order: first the count of entries,
    /// then each key-value pair sorted by key bytes.
    ///
    /// Format: `[count (u64)][key1 (string)][value1 (bytes)][key2 (string)][value2 (bytes)]...`
    #[must_use]
    pub fn add_map_string_to_bytes(mut self, map: &BTreeMap<String, Vec<u8>>) -> Self {
        // Add count
        self = self.add_u64(map.len() as u64);

        // BTreeMap iterates in sorted order by key, but we need to sort by key bytes
        // to match the JS SDK's behavior (which sorts by UTF-8 byte comparison)
        let mut pairs: Vec<_> = map.iter().collect();
        pairs.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

        for (key, value) in pairs {
            self = self.add_string(key);
            self = self.add_bytes(value);
        }

        self
    }

    /// Returns the raw signable message: `tagHash || tagHash || values`.
    ///
    /// Use this when you need to pass the message to a signing function
    /// that will hash it before signing.
    pub fn signable_message(self) -> Vec<u8> {
        let mut msg = Vec::with_capacity(64 + self.values.len());
        msg.extend_from_slice(&self.tag_hash);
        msg.extend_from_slice(&self.tag_hash);
        msg.extend_from_slice(&self.values);
        msg
    }

    /// Computes and returns the final SHA256 hash.
    ///
    /// This is equivalent to `SHA256(signable_message())`.
    pub fn hash(self) -> sha256::Hash {
        sha256::Hash::hash(&self.signable_message())
    }

    /// Computes the tag hash from a hierarchical tag.
    ///
    /// Format: For each component, `[8-byte BE length][UTF-8 bytes]`
    fn compute_tag_hash(tag: &[&str]) -> [u8; 32] {
        let mut engine = sha256::HashEngine::default();

        for component in tag {
            let component_bytes = component.as_bytes();
            let length_bytes = (component_bytes.len() as u64).to_be_bytes();
            engine.input(&length_bytes);
            engine.input(component_bytes);
        }

        sha256::Hash::from_engine(engine).to_byte_array()
    }

    /// Writes a value with an 8-byte big-endian length prefix.
    fn write_length_prefixed(&mut self, bytes: &[u8]) {
        let length_bytes = (bytes.len() as u64).to_be_bytes();
        self.values.extend_from_slice(&length_bytes);
        self.values.extend_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_different_tags_produce_different_hashes() {
        let hash1 = TaggedHasher::new(&["spark", "a"])
            .add_bytes(&[1, 2, 3])
            .hash();
        let hash2 = TaggedHasher::new(&["spark", "b"])
            .add_bytes(&[1, 2, 3])
            .hash();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_empty_vs_nonempty_bytes() {
        let hash_none = TaggedHasher::new(&["test"]).hash();
        let hash_empty = TaggedHasher::new(&["test"]).add_bytes(&[]).hash();
        assert_ne!(hash_none, hash_empty);
    }

    #[test]
    fn test_order_matters() {
        let hash1 = TaggedHasher::new(&["test"])
            .add_bytes(&[1])
            .add_bytes(&[2])
            .hash();
        let hash2 = TaggedHasher::new(&["test"])
            .add_bytes(&[2])
            .add_bytes(&[1])
            .hash();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_map_ordering() {
        let mut map = BTreeMap::new();
        map.insert("b".to_string(), vec![2]);
        map.insert("a".to_string(), vec![1]);

        let hash1 = TaggedHasher::new(&["test"])
            .add_map_string_to_bytes(&map)
            .hash();

        // Same map, different insertion order should produce same hash
        let mut map2 = BTreeMap::new();
        map2.insert("a".to_string(), vec![1]);
        map2.insert("b".to_string(), vec![2]);

        let hash2 = TaggedHasher::new(&["test"])
            .add_map_string_to_bytes(&map2)
            .hash();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_add_string() {
        let hash1 = TaggedHasher::new(&["test"]).add_string("hello").hash();
        let hash2 = TaggedHasher::new(&["test"]).add_bytes(b"hello").hash();
        // add_string and add_bytes should produce the same result for ASCII strings
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_add_u64() {
        let hash1 = TaggedHasher::new(&["test"]).add_u64(12345).hash();
        let hash2 = TaggedHasher::new(&["test"]).add_u64(12345).hash();
        assert_eq!(hash1, hash2);

        let hash3 = TaggedHasher::new(&["test"]).add_u64(12346).hash();
        assert_ne!(hash1, hash3);
    }
}
