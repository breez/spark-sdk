use std::str::FromStr;
use std::sync::Arc;

use bip39::Mnemonic;
use bitcoin::bip32::ChildNumber;
use bitcoin::bip32::DerivationPath;
use bitcoin::bip32::Xpriv;
use bitcoin::key::Secp256k1;
use bitcoin::secp256k1::PublicKey;
use spark_wallet::DefaultSigner;
use spark_wallet::DefaultSignerError;
use spark_wallet::Network;
use spark_wallet::SparkAddress;

use crate::NETWORK;
use crate::flashnet::SignerStore;
use crate::flashnet::SimpleSignerStore;

const PURPOSE: u32 = 8797555;

const RECEIVER_MNEMONIC: &str =
    "artist kiwi silk miss peasant finger total fit topple stamp abandon bean";
const SENDER_MNEMONIC: &str =
    "praise dose reward flat confirm wheel crush hood kiss ability differ boss";

const RECEIVER_IDENTITY_PUBKEY: &str =
    "039bafa7427597705fe65dfe7ff39db0194d682d0cb49031d601cc37dd8d71e51b";
const SENDER_IDENTITY_PUBKEY: &str =
    "03fbd7dd948d003d20662ce0a38a75355cad5f588719a3f7f4c81184a5a7df1eec";

pub(crate) async fn init_store() -> (Arc<dyn SignerStore>, PublicKey, PublicKey) {
    let store = Arc::new(SimpleSignerStore::new());
    // Insert the sender seed
    let mnemonic = Mnemonic::from_str(SENDER_MNEMONIC).unwrap();
    let seed = mnemonic.to_seed("");
    let sender_public_key = create_user_keypair(&seed, NETWORK);
    _ = store.insert_seed(sender_public_key, seed.to_vec()).unwrap();

    // Insert the receiver seed
    let mnemonic = Mnemonic::from_str(RECEIVER_MNEMONIC).unwrap();
    let seed = mnemonic.to_seed("");
    let receiver_public_key = create_user_keypair(&seed, NETWORK);
    _ = store
        .insert_seed(receiver_public_key, seed.to_vec())
        .unwrap();

    assert_eq!(sender_public_key.to_string(), SENDER_IDENTITY_PUBKEY);
    assert_eq!(receiver_public_key.to_string(), RECEIVER_IDENTITY_PUBKEY);

    (store, sender_public_key, receiver_public_key)
}

pub(crate) fn create_user_keypair(seed: &[u8], network: Network) -> PublicKey {
    let master_key = Xpriv::new_master(network, seed).unwrap();
    let secp = Secp256k1::new();
    let identity_key = master_key
        .derive_priv(&secp, &identity_derivation_path(network))
        .unwrap()
        .private_key;
    identity_key.keypair(&secp).public_key()
}

pub(crate) fn user_signer(
    store: Arc<dyn SignerStore>,
    public_key: &PublicKey,
) -> Result<DefaultSigner, DefaultSignerError> {
    let seed = store.get_seed(public_key).unwrap();
    let signer = DefaultSigner::new(&seed, NETWORK);
    signer
}

pub(crate) fn spark_address(public_key: PublicKey) -> SparkAddress {
    SparkAddress::new(public_key, NETWORK, None, None)
}

fn identity_derivation_path(network: Network) -> DerivationPath {
    DerivationPath::from(vec![
        purpose(),
        coin_type(network),
        ChildNumber::from_hardened_idx(0).expect("Hardened zero is invalid"),
    ])
}

fn coin_type(network: Network) -> ChildNumber {
    let coin_type: u32 = match network {
        Network::Regtest => 0,
        _ => 1,
    };
    ChildNumber::from_hardened_idx(coin_type)
        .unwrap_or_else(|_| panic!("Hardened coin type {coin_type} is invalid"))
}

fn purpose() -> ChildNumber {
    ChildNumber::from_hardened_idx(PURPOSE)
        .unwrap_or_else(|_| panic!("Hardened purpose {PURPOSE} is invalid"))
}
