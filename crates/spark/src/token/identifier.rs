//! Token identifier derivation, ported from `ComputeTokenIdentifier` in
//! `spark/common/token_metadata.go` (github.com/buildonspark/spark).
//!
//! The identifier is a hash over the create fields, issuer key included, so a
//! metadata record whose identifier recomputes from its own fields cannot pair
//! one token's identifier with another token's issuer.

use bitcoin::hashes::{Hash, HashEngine, sha256};
use bitcoin::secp256k1::PublicKey;

use super::{TokenMetadata, bech32m_decode_token_id};
use crate::Network;

/// Version field of the identifier format that covers extra metadata.
const EXTENSIONS_VERSION: u64 = 2;

impl TokenMetadata {
    /// Whether this is the token `issuer` created: it names `issuer` and its identifier derives
    /// from that key.
    pub fn is_issued_by(&self, issuer: &PublicKey, network: Network) -> bool {
        self.issuer_public_key == *issuer && self.has_valid_identifier(network)
    }

    /// Whether [`TokenMetadata::identifier`] is the one the operators derive from the other fields.
    /// Tokens in an identifier format newer than this SDK fail too.
    fn has_valid_identifier(&self, network: Network) -> bool {
        let Ok(identifier) = bech32m_decode_token_id(&self.identifier, Some(network)) else {
            return false;
        };
        match self.extra_metadata.as_deref() {
            Some(extra) if !extra.is_empty() => {
                self.identifier_with_extensions(network) == Some(identifier)
            }
            // The operator picks the format by whether extra metadata is absent or empty, which
            // a stored or relayed record may not preserve. Both formats hash the issuer key.
            _ => {
                self.identifier_v1(network).as_ref() == Some(&identifier)
                    || self.identifier_with_extensions(network).as_ref() == Some(&identifier)
            }
        }
    }

    fn identifier_v1(&self, network: Network) -> Option<Vec<u8>> {
        let decimals = u8::try_from(self.decimals).ok()?;
        let mut engine = sha256::Hash::engine();
        for part in [
            &[1][..],
            &self.issuer_public_key.serialize(),
            self.name.as_bytes(),
            self.ticker.as_bytes(),
            &[decimals],
            &self.max_supply.to_be_bytes(),
            &[u8::from(self.is_freezable)],
            &network_magic(network).to_be_bytes(),
        ] {
            engine.input(sha256::Hash::hash(part).as_byte_array());
        }
        // Creation layer 1 is L1, layer 2 is Spark followed by the creation entity key.
        let layer = match self.creation_entity_public_key {
            None => sha256::Hash::hash(&[1]),
            Some(key) => sha256::Hash::hash(&[&[2][..], &key.serialize()].concat()),
        };
        engine.input(layer.as_byte_array());
        Some(sha256::Hash::from_engine(engine).to_byte_array().to_vec())
    }

    /// Object hash of the operator's `UnencodedTokenIdentifier` message, field numbers as in
    /// `spark_token_internal.proto`. Scalars at their default value are left out, as the
    /// protobuf hasher does.
    fn identifier_with_extensions(&self, network: Network) -> Option<Vec<u8>> {
        if self.decimals > 255 {
            return None;
        }
        let creation_entity_public_key = self
            .creation_entity_public_key
            .map_or([0; 33], |key| key.serialize());
        let mut fields = vec![
            (1, hash_int(EXTENSIONS_VERSION)),
            (2, hash_bytes(&self.issuer_public_key.serialize())),
        ];
        if !self.name.is_empty() {
            fields.push((3, hash_tagged(b"u", self.name.as_bytes())));
        }
        if !self.ticker.is_empty() {
            fields.push((4, hash_tagged(b"u", self.ticker.as_bytes())));
        }
        if self.decimals != 0 {
            fields.push((5, hash_int(u64::from(self.decimals))));
        }
        fields.push((6, hash_bytes(&self.max_supply.to_be_bytes())));
        if self.is_freezable {
            fields.push((7, hash_tagged(b"b", b"1")));
        }
        let proto_network = u64::try_from(network.to_proto_network() as i32).ok()?;
        fields.push((8, hash_int(proto_network)));
        fields.push((9, hash_bytes(&creation_entity_public_key)));
        if let Some(extra) = self.extra_metadata.as_deref().filter(|m| !m.is_empty()) {
            fields.push((10, hash_bytes(extra)));
        }

        let mut engine = sha256::Hash::engine();
        engine.input(b"d");
        for (number, value) in fields {
            engine.input(&hash_int(number));
            engine.input(&value);
        }
        Some(sha256::Hash::from_engine(engine).to_byte_array().to_vec())
    }
}

/// Network magic as btcd's `wire.BitcoinNet`, which the identifier hashes big-endian.
fn network_magic(network: Network) -> u32 {
    match network {
        Network::Mainnet => 0xd9b4_bef9,
        Network::Testnet => 0x0709_110b,
        Network::Regtest => 0xdab5_bffa,
        Network::Signet => 0x40cf_030a,
    }
}

fn hash_tagged(tag: &[u8], value: &[u8]) -> [u8; 32] {
    sha256::Hash::hash(&[tag, value].concat()).to_byte_array()
}

fn hash_int(value: u64) -> [u8; 32] {
    hash_tagged(b"i", &value.to_be_bytes())
}

fn hash_bytes(value: &[u8]) -> [u8; 32] {
    hash_tagged(b"r", value)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;

    use bitcoin::secp256k1::PublicKey;
    use macros::{async_test_all, test_all};
    use platform_utils::time::SystemTime;

    use crate::Network;
    use crate::operator::testing::unroutable_operator_pool;
    use crate::signer::{SparkSigner, SparkSignerAdapter, create_test_signer};
    use crate::token::tests::create_token_outputs;
    use crate::token::{
        InMemoryTokenOutputStore, SynchronousTokenOutputService, TokenMetadata, TokenOutputStore,
        TokenOutputs, TokenService, TokensConfig, bech32m_encode_token_id,
    };

    #[cfg(feature = "browser-tests")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const OTHER_KEY: &str = "0345b806679a5e63159584db91fec038cffd2ef59cee031abe92e2f30bf0642175";

    fn pk(hex: &str) -> PublicKey {
        PublicKey::from_str(hex).unwrap()
    }

    /// `RaccoonCoin` is from `TestActualProduction*TokenIdentifier` in
    /// `spark/common/token_metadata_test.go`. `VectorCoin`, which has extra metadata, was
    /// created on Lightspark's regtest operators.
    fn operator_tokens() -> Vec<TokenMetadata> {
        let raccoon = |identifier: &str, creation_entity_public_key| TokenMetadata {
            identifier: bech32m_encode_token_id(
                &hex::decode(identifier).unwrap(),
                Network::Regtest,
            )
            .unwrap(),
            issuer_public_key: pk(
                "036898ed2b633947f0994b8952fa06da2cfc7d1ee003fcf2cc076752b9ad3b3691",
            ),
            name: "RaccoonCoin".to_string(),
            ticker: "RCC".to_string(),
            decimals: 10,
            max_supply: 0x9c3f,
            is_freezable: false,
            creation_entity_public_key,
            extra_metadata: None,
        };
        vec![
            raccoon(
                "f1ca1e65691d0f65132ce24608594aaccd741e323056c97407a9f625b0ee4251",
                None,
            ),
            raccoon(
                "8b5fde73c803f6ef5c819ae94ddd035f02bee63555a08fc94f6851e289b46a1b",
                Some(pk(OTHER_KEY)),
            ),
            TokenMetadata {
                identifier: "btknrt1m6skvhcmu3c7jr7tumwe6q2vsudzp6w6swgyj6xyw87dx2mjf2rq2eces8"
                    .to_string(),
                issuer_public_key: pk(
                    "03890a5694eaf8a8340960344c94f6e24aca8492d64a99a23fc32d2e59b016c7d3",
                ),
                name: "VectorCoin".to_string(),
                ticker: "VEC".to_string(),
                decimals: 0,
                max_supply: 0,
                is_freezable: true,
                creation_entity_public_key: Some(pk(
                    "0205fe807e8fe1f368df955cc291f16d840b7f28374b0ed80b80c3e2e0921a0674",
                )),
                extra_metadata: Some(b"{k:1}".to_vec()),
            },
        ]
    }

    #[test_all]
    fn identifiers_match_the_operators() {
        for metadata in operator_tokens() {
            assert!(
                metadata.has_valid_identifier(Network::Regtest),
                "{metadata:?}"
            );
            assert!(!metadata.has_valid_identifier(Network::Mainnet));

            let mut other_decimals = metadata.clone();
            other_decimals.decimals += 1;
            assert!(!other_decimals.has_valid_identifier(Network::Regtest));
        }
    }

    #[test_all]
    fn network_magic_matches_bitcoin_network() {
        for (network, bitcoin_network) in [
            (Network::Mainnet, bitcoin::Network::Bitcoin),
            (Network::Testnet, bitcoin::Network::Testnet),
            (Network::Regtest, bitcoin::Network::Regtest),
            (Network::Signet, bitcoin::Network::Signet),
        ] {
            let mut wire = bitcoin_network.magic().to_bytes();
            wire.reverse();
            assert_eq!(super::network_magic(network).to_be_bytes(), wire);
        }
    }

    #[test_all]
    fn issued_by_needs_issuer_key_and_identifier() {
        let own = operator_tokens().remove(0);
        let issuer = own.issuer_public_key;
        assert!(own.is_issued_by(&issuer, Network::Regtest));
        // Another issuer's token, as is and relabelled with this issuer's key.
        let mut other = operator_tokens().remove(2);
        assert!(!other.is_issued_by(&issuer, Network::Regtest));
        other.issuer_public_key = issuer;
        assert!(!other.is_issued_by(&issuer, Network::Regtest));
    }

    /// The token output store holding another token's outputs under metadata that names the
    /// wallet as issuer, as a coordinator can report it.
    #[async_test_all]
    async fn issuer_token_is_not_another_token_labelled_as_the_wallets() {
        let signer: Arc<dyn SparkSigner> =
            Arc::new(SparkSignerAdapter::new(Arc::new(create_test_signer())));
        let identity = signer.get_identity_public_key().await.unwrap();
        let store = Arc::new(InMemoryTokenOutputStore::default());
        let operator_pool = unroutable_operator_pool(&signer).await;
        let service = TokenService::new(
            Arc::new(SynchronousTokenOutputService::new(
                Network::Regtest,
                Arc::clone(&operator_pool),
                store.clone(),
                Arc::clone(&signer),
            )),
            signer,
            operator_pool,
            Network::Regtest,
            TokensConfig {
                expected_withdraw_bond_sats: 10_000,
                expected_withdraw_relative_block_locktime: 1_000,
                transaction_validity_duration_seconds: 180,
                max_tx_inputs: 500,
            },
            None,
        );

        let mut metadata = operator_tokens().remove(0);
        let mut outputs = create_token_outputs(2, vec![1_000]).outputs;
        for output in &mut outputs {
            output.output.token_identifier = metadata.identifier.clone();
        }
        metadata.issuer_public_key = identity;
        store
            .set_tokens_outputs(&TokenOutputs::single(metadata, outputs), SystemTime::now())
            .await
            .unwrap();

        assert!(service.get_issuer_token_metadata().await.is_err());
    }
}
