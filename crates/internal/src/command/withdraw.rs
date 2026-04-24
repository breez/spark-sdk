use std::str::FromStr;

use bitcoin::{
    self, Address, Amount, OutPoint, PrivateKey, Psbt, TxOut, Txid, Witness,
    bip32::{DerivationPath, Xpriv},
    consensus::encode::serialize_hex,
    ecdsa::Signature,
    hashes::{Hash, sha256},
    key::Secp256k1,
    secp256k1::{PublicKey, SecretKey},
    sighash::{self, SighashCache},
};
use clap::Subcommand;
use spark_wallet::{
    CpfpInput, Network, SparkWallet, SparkWalletError, TreeNodeId, is_ephemeral_anchor_output,
};

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum ExitSpeed {
    Fast,
    Medium,
    Slow,
}

impl From<ExitSpeed> for spark_wallet::ExitSpeed {
    fn from(speed: ExitSpeed) -> Self {
        match speed {
            ExitSpeed::Fast => spark_wallet::ExitSpeed::Fast,
            ExitSpeed::Medium => spark_wallet::ExitSpeed::Medium,
            ExitSpeed::Slow => spark_wallet::ExitSpeed::Slow,
        }
    }
}

struct ParsedCpfpInput(CpfpInput);

impl std::str::FromStr for ParsedCpfpInput {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 4 && parts.len() != 5 {
            return Err(
                "Invalid format, expected txid:vout:value:pubkey[:type] where type is p2wpkh or p2tr"
                    .into(),
            );
        }

        let txid = Txid::from_str(parts[0])?;
        let vout = parts[1].parse::<u32>()?;
        let value = parts[2].parse::<u64>()?;
        let pubkey_bytes = hex::decode(parts[3])?;
        let pubkey = PublicKey::from_slice(&pubkey_bytes)?;

        let utxo_type = if parts.len() == 5 { parts[4] } else { "p2wpkh" };

        let (script_pubkey, signed_input_weight) = match utxo_type {
            "p2wpkh" => {
                let script = bitcoin::Address::p2wpkh(
                    &bitcoin::CompressedPublicKey(pubkey),
                    bitcoin::Network::Bitcoin,
                )
                .script_pubkey();
                (script, 272)
            }
            "p2tr" => {
                let secp = bitcoin::key::Secp256k1::new();
                let (xonly, _) = pubkey.x_only_public_key();
                let script = bitcoin::Address::p2tr(&secp, xonly, None, bitcoin::Network::Bitcoin)
                    .script_pubkey();
                (script, 230)
            }
            other => {
                return Err(format!("Unknown UTXO type '{other}', expected p2wpkh or p2tr").into());
            }
        };

        Ok(ParsedCpfpInput(CpfpInput {
            outpoint: OutPoint { txid, vout },
            witness_utxo: TxOut {
                value: Amount::from_sat(value),
                script_pubkey,
            },
            signed_input_weight,
        }))
    }
}

#[derive(Clone, Debug, Subcommand)]
pub enum WithdrawCommand {
    /// Fetch the current coop exit fee quote.
    FetchFeeQuote {
        withdrawal_address: String,
        amount_sats: Option<u64>,
    },
    /// Perform a coop exit.
    CoopExit {
        withdrawal_address: String,
        exit_speed: ExitSpeed,
        amount_sats: Option<u64>,
    },
    /// Prepare a unilateral exit package.
    UnilateralExit {
        /// Fee rate in sats/vbyte.
        fee_rate: u64,
        /// The leaf IDs of the tree nodes to unilateral exit. Defaults to all leaves.
        #[clap(short, long = "leaf")]
        leaf_ids: Vec<TreeNodeId>,
        /// Hex-encoded UTXOs "[txid:vout:value:pubkey]" used to pay fees for the unilateral exit.
        #[clap(short, long = "utxo")]
        utxos: Vec<String>,
        /// Optional hex-encoded private key to sign PSBTs
        #[clap(short)]
        signing_key: Option<String>,
    },
}

pub async fn handle_command(
    network: Network,
    wallet: &SparkWallet,
    command: WithdrawCommand,
    seed: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        WithdrawCommand::FetchFeeQuote {
            withdrawal_address,
            amount_sats,
        } => {
            let fee_quote = wallet
                .fetch_coop_exit_fee_quote(&withdrawal_address, amount_sats)
                .await?;
            println!("{}", serde_json::to_string_pretty(&fee_quote)?);
        }
        WithdrawCommand::CoopExit {
            withdrawal_address,
            exit_speed,
            amount_sats,
        } => {
            let fee_quote = wallet
                .fetch_coop_exit_fee_quote(&withdrawal_address, amount_sats)
                .await?;

            let result = wallet
                .withdraw(
                    &withdrawal_address,
                    amount_sats,
                    exit_speed.into(),
                    fee_quote,
                    None,
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        WithdrawCommand::UnilateralExit {
            fee_rate,
            leaf_ids,
            utxos,
            signing_key,
        } => {
            let signing_key = signing_key
                .map(|pk| SecretKey::from_str(&pk))
                .transpose()?
                .map(|pk| PrivateKey::new(pk, network));

            let inputs = utxos
                .into_iter()
                .map(|s| ParsedCpfpInput::from_str(&s).map(|wrapper| wrapper.0))
                .collect::<Result<_, _>>()?;
            let all_leaf_tx_cpfp_psbts = wallet
                .unilateral_exit(fee_rate, leaf_ids, inputs, None)
                .await?;

            for leaf_tx_cpfp_psbts in &all_leaf_tx_cpfp_psbts {
                println!();
                println!("Leaf ID: {}", leaf_tx_cpfp_psbts.leaf_id);
                println!();

                let total_txs = leaf_tx_cpfp_psbts.tx_cpfp_psbts.len();
                for (index, tx_cpfp_psbt) in leaf_tx_cpfp_psbts.tx_cpfp_psbts.iter().enumerate() {
                    let index_str = format!("{}. ", index + 1);
                    let index_spaces = " ".repeat(index_str.len());

                    // Order: Node TX(s), Leaf TX, Refund TX
                    // The last item is always the Refund TX, second-to-last is Leaf TX
                    let is_refund_tx = index == total_txs - 1;
                    let is_leaf_tx = index == total_txs - 2;
                    let tx_type = if is_refund_tx {
                        "Refund TX"
                    } else if is_leaf_tx {
                        "Leaf TX"
                    } else {
                        "Node TX"
                    };

                    let txid = tx_cpfp_psbt.parent_tx.compute_txid();
                    let tx_hex = serialize_hex(&tx_cpfp_psbt.parent_tx);
                    println!("{index_str}{tx_type} ID: {txid}");
                    println!("{index_spaces}{tx_type}: {tx_hex}");

                    let mut psbt = tx_cpfp_psbt.child_psbt.clone();
                    let psbt_hex = psbt.serialize_hex();
                    println!("{index_spaces}PSBT (unsigned): {psbt_hex}");

                    if let Some(signing_key) = &signing_key {
                        sign_psbt(&mut psbt, signing_key)?;

                        let signed_tx = psbt.extract_tx()?;
                        let signed_txid = signed_tx.compute_txid();
                        let signed_tx_hex = serialize_hex(&signed_tx);
                        println!("{index_spaces}PSBT signed TX ID: {signed_txid}");
                        println!("{index_spaces}PSBT signed TX: {signed_tx_hex}");
                    }

                    // Display CSV timelock for refund transaction
                    if is_refund_tx && let Some(input) = tx_cpfp_psbt.parent_tx.input.first() {
                        let sequence = input.sequence.to_consensus_u32();
                        // CSV uses the lower 16 bits for the relative lock value
                        // Bit 22 (0x00400000) indicates blocks vs time
                        if sequence & 0x00400000 == 0 {
                            let csv_blocks = sequence & 0xFFFF;
                            println!(
                                "{index_spaces}Timelock: {} blocks after Leaf TX confirms",
                                csv_blocks
                            );
                        }
                    }

                    // Independent derivation path verification for refund TX
                    if is_refund_tx {
                        println!();
                        println!();
                        verify_refund_derivation_path(
                            seed,
                            network,
                            &leaf_tx_cpfp_psbts.leaf_id,
                            &tx_cpfp_psbt.parent_tx,
                        )?;
                    }

                    println!();
                }
                println!();
            }
            println!("For each leaf, broadcast one-by-one each TX and signed PSBT.");
            println!(
                "TXs and signed PSBTs should be broadcasted in the order they appear: Node TX(s) > Leaf TX > Refund TX"
            );
            println!(
                "The Refund TX can only be broadcast after its timelock expires (blocks after Leaf TX confirms)."
            );
            println!(
                "Use the taproot descriptor shown for each refund TX to sweep funds into any Bitcoin wallet."
            );
        }
    }

    Ok(())
}

/// Independently derives keys from the seed and checks which derivation path
/// matches the refund TX output address. This verifies the derivation path
/// without relying on DefaultSigner or SparkWallet.
fn verify_refund_derivation_path(
    seed: &[u8],
    network: Network,
    leaf_id: &TreeNodeId,
    refund_tx: &bitcoin::Transaction,
) -> Result<(), Box<dyn std::error::Error>> {
    let secp = Secp256k1::new();
    let btc_network: bitcoin::Network = network.into();
    let master = Xpriv::new_master(btc_network, seed)?;

    let account: u32 = match network {
        Network::Regtest => 0,
        _ => 1,
    };
    let coin_type: u32 = match network {
        Network::Mainnet => 0,
        _ => 1,
    };

    let refund_script = &refund_tx.output[0].script_pubkey;
    let refund_script_bytes = refund_script.as_bytes();
    // P2TR scriptPubkey: OP_1 (0x51) + PUSH32 (0x20) + 32-byte x-only pubkey
    if refund_script_bytes.len() < 34 || refund_script_bytes[0] != 0x51 {
        println!("Refund output is not P2TR, skipping verification");
        return Ok(());
    }

    // Node signing key child index: sha256(leaf_id)[0..4] % 2^31
    let hash = sha256::Hash::hash(leaf_id.to_string().as_bytes());
    let hash_bytes: &[u8] = hash.as_ref();
    let signing_child_index = u32::from_be_bytes(hash_bytes[..4].try_into().unwrap()) % 0x8000_0000;

    let candidates = [
        format!("m/8797555'/{account}'/0'"),
        format!("m/8797555'/{account}'"),
        format!("m/8797555'/{account}'/1'/{signing_child_index}'"),
        format!("m/86'/{coin_type}'/{account}'/0/0"),
    ];
    let labels = [
        "identity key",
        "identity master",
        "node signing key",
        "BIP86 taproot",
    ];

    let mut found_match = false;
    for (path_str, label) in candidates.iter().zip(labels.iter()) {
        let path: DerivationPath = path_str.parse()?;
        let derived = master.derive_priv(&secp, &path)?;
        let pubkey = derived.private_key.public_key(&secp);
        let (xonly, _parity) = pubkey.x_only_public_key();
        let addr = Address::p2tr(&secp, xonly, None, btc_network);
        if *refund_script == addr.script_pubkey() {
            let wif = PrivateKey::new(derived.private_key, btc_network);
            println!("Derivation path: {path_str} ({label})");
            println!("Refund address:  {addr}");
            println!("Descriptor:      tr({wif})");
            found_match = true;
            break;
        }
    }

    if !found_match {
        println!("WARNING: No candidate derivation path matched the refund output");
    }

    Ok(())
}

fn sign_psbt(psbt: &mut Psbt, signing_key: &PrivateKey) -> Result<(), SparkWalletError> {
    let secp = Secp256k1::new();
    let pubkey = signing_key.public_key(&secp);

    // Collect all prevouts for taproot sighash computation
    let prevouts: Vec<TxOut> = psbt
        .inputs
        .iter()
        .map(|input| input.witness_utxo.clone().unwrap_or(TxOut::NULL))
        .collect();

    let mut cache = SighashCache::new(&psbt.unsigned_tx);
    let mut ecdsa_signatures = vec![];
    let mut taproot_indices = vec![];
    let mut anchor_index = None;

    for (i, input) in psbt.inputs.iter().enumerate() {
        if let Some(tx_out) = &input.witness_utxo {
            if is_ephemeral_anchor_output(tx_out) {
                anchor_index = Some(i);
            } else if tx_out.script_pubkey.is_p2tr() {
                taproot_indices.push(i);
            } else {
                let (msg, ecdsa_type) = psbt
                    .sighash_ecdsa(i, &mut cache)
                    .map_err(|e| SparkWalletError::Generic(e.to_string()))?;
                let sig = secp.sign_ecdsa(&msg, &signing_key.inner);
                let signature = Signature {
                    signature: sig,
                    sighash_type: ecdsa_type,
                };
                ecdsa_signatures.push((i, pubkey, signature));
            }
        }
    }

    // Apply ECDSA signatures
    for (i, pubkey, signature) in ecdsa_signatures {
        psbt.inputs[i].partial_sigs.insert(pubkey, signature);
    }

    // Sign taproot inputs with Schnorr key-path spend
    if !taproot_indices.is_empty() {
        let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &signing_key.inner);
        let prevouts_ref = sighash::Prevouts::All(&prevouts);
        for i in taproot_indices {
            let sighash = cache
                .taproot_key_spend_signature_hash(
                    i,
                    &prevouts_ref,
                    sighash::TapSighashType::Default,
                )
                .map_err(|e| SparkWalletError::Generic(e.to_string()))?;
            let msg = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
            let schnorr_sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
            let tap_sig = bitcoin::taproot::Signature {
                signature: schnorr_sig,
                sighash_type: sighash::TapSighashType::Default,
            };
            psbt.inputs[i].tap_key_sig = Some(tap_sig);
        }
    }

    // Set an empty witness for the anchor input
    if let Some(anchor_index) = anchor_index {
        psbt.inputs[anchor_index].final_script_witness = Some(Witness::new());
    }
    Ok(())
}
