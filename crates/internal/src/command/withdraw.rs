use std::str::FromStr;

use bitcoin::{
    self, Address, Amount, CompressedPublicKey, OutPoint, PrivateKey, Psbt, TxOut, Txid, Witness,
    bip32::{DerivationPath, Xpriv},
    consensus::encode::serialize_hex,
    ecdsa::Signature,
    hashes::{Hash, sha256},
    key::Secp256k1,
    secp256k1::{PublicKey, SecretKey},
    sighash::SighashCache,
};
use clap::Subcommand;
use spark_wallet::{
    CpfpInput, ExitLeafSelection, ExitTxKind, Network, SparkWallet, SparkWalletError, TreeNodeId,
    build_unilateral_exit, is_ephemeral_anchor_output, p2wpkh_input_weight,
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

/// Parses a `txid:vout:value:pubkey` funding UTXO (hex pubkey) into a P2WPKH
/// [`CpfpInput`] carrying the exact signed input weight so fees stay exact.
fn parse_p2wpkh_funding(
    s: &str,
    network: Network,
) -> Result<CpfpInput, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 4 {
        return Err("Invalid format, expected txid:vout:value:pubkey".into());
    }

    let txid = Txid::from_str(parts[0])?;
    let vout = parts[1].parse::<u32>()?;
    let value = parts[2].parse::<u64>()?;
    let pubkey = PublicKey::from_slice(&hex::decode(parts[3])?)?;
    let btc_network: bitcoin::Network = network.into();
    let script_pubkey = Address::p2wpkh(&CompressedPublicKey(pubkey), btc_network).script_pubkey();

    Ok(CpfpInput {
        outpoint: OutPoint { txid, vout },
        witness_utxo: TxOut {
            value: Amount::from_sat(value),
            script_pubkey,
        },
        signed_input_weight: p2wpkh_input_weight().to_wu(),
    })
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
        /// Fee rate in satoshis per 1000 weight units (sat/kW).
        fee_rate_sat_per_kw: u64,
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
            println!("{result:#?}");
        }
        WithdrawCommand::UnilateralExit {
            fee_rate_sat_per_kw,
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
                .map(|s| parse_p2wpkh_funding(&s, network))
                .collect::<Result<Vec<_>, _>>()?;
            // No `--leaf` flags means exit every available leaf (Auto).
            let selection = if leaf_ids.is_empty() {
                ExitLeafSelection::Auto
            } else {
                ExitLeafSelection::Specific(leaf_ids)
            };
            let prepared = wallet
                .prepare_unilateral_exit_plan(
                    fee_rate_sat_per_kw,
                    selection,
                    inputs,
                    // Sweep destination is a P2TR address (34-byte scriptPubKey);
                    // only used here to size the fan-out fee.
                    34,
                )
                .await?;
            // No chain access here, so pass no observations: the exit resolves to
            // a fresh full build (nothing recognized as already on-chain).
            let exit = build_unilateral_exit(&prepared, &[], fee_rate_sat_per_kw)?;

            if let Some(fan_out) = &exit.fan_out
                && let Some(psbt) = &fan_out.to_sign
            {
                println!();
                println!("Fan-out TX (broadcast before any branch):");
                let mut psbt = psbt.clone();
                println!("PSBT (unsigned): {}", psbt.serialize_hex());
                if let Some(signing_key) = &signing_key {
                    sign_psbt(&mut psbt, signing_key)?;
                    let signed_tx = psbt.extract_tx()?;
                    println!("Signed TX ID: {}", signed_tx.compute_txid());
                    println!("Signed TX: {}", serialize_hex(&signed_tx));
                }
                println!();
            }

            for branch in &exit.branches {
                println!();
                println!("Leaf ID: {}", branch.leaf_id);
                println!();

                let total_txs = branch.txs.len();
                for (index, tx) in branch.txs.iter().enumerate() {
                    let index_str = format!("{}. ", index + 1);
                    let index_spaces = " ".repeat(index_str.len());

                    // Order: Node TX(s), Leaf TX, Refund TX. The last is the Refund
                    // TX, second-to-last the Leaf TX (the leaf's own node_tx).
                    let is_refund_tx = tx.kind == ExitTxKind::Refund;
                    let is_leaf_tx = index == total_txs.saturating_sub(2);
                    let tx_type = if is_refund_tx {
                        "Refund TX"
                    } else if is_leaf_tx {
                        "Leaf TX"
                    } else {
                        "Node TX"
                    };

                    let txid = tx.txid;
                    let tx_hex = serialize_hex(&tx.base_tx);
                    println!("{index_str}{tx_type} ID: {txid}");
                    println!("{index_spaces}{tx_type}: {tx_hex}");

                    if let Some(child) = &tx.to_sign {
                        let mut psbt = child.clone();
                        println!("{index_spaces}PSBT (unsigned): {}", psbt.serialize_hex());
                        if let Some(signing_key) = &signing_key {
                            sign_psbt(&mut psbt, signing_key)?;
                            let signed_tx = psbt.extract_tx()?;
                            println!(
                                "{index_spaces}PSBT signed TX ID: {}",
                                signed_tx.compute_txid()
                            );
                            println!(
                                "{index_spaces}PSBT signed TX: {}",
                                serialize_hex(&signed_tx)
                            );
                        }
                    }

                    // Display the CSV timelock for the refund transaction.
                    if is_refund_tx && let Some(blocks) = tx.csv_timelock_blocks {
                        println!("{index_spaces}Timelock: {blocks} blocks after Leaf TX confirms");
                    }

                    // Independent derivation-path verification for the refund TX.
                    if is_refund_tx {
                        println!();
                        println!();
                        verify_refund_derivation_path(seed, network, &branch.leaf_id, &tx.base_tx)?;
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
    let mut cache = SighashCache::new(&psbt.unsigned_tx);
    let mut signatures = vec![];
    let mut anchor_index = None;
    let secp = Secp256k1::new();
    let pubkey = signing_key.public_key(&secp);
    // Sign inputs where the witness utxo is a non anchor output
    for (i, input) in psbt.inputs.iter().enumerate() {
        if let Some(tx_out) = &input.witness_utxo {
            if is_ephemeral_anchor_output(tx_out) {
                anchor_index = Some(i);
            } else {
                let (msg, ecdsa_type) = psbt
                    .sighash_ecdsa(i, &mut cache)
                    .map_err(|e| SparkWalletError::Generic(e.to_string()))?;
                let sig = secp.sign_ecdsa(&msg, &signing_key.inner);
                let signature = Signature {
                    signature: sig,
                    sighash_type: ecdsa_type,
                };
                signatures.push((i, pubkey, signature));
            }
        }
    }
    // Finalize each signed input with its P2WPKH witness stack (sig, pubkey).
    // `extract_tx` reads only `final_script_witness`, so leaving the signature
    // in `partial_sigs` would extract an unsigned funding input.
    for (i, pubkey, signature) in signatures.into_iter() {
        let mut witness = Witness::new();
        witness.push(signature.to_vec());
        witness.push(pubkey.to_bytes());
        psbt.inputs[i].final_script_witness = Some(witness);
    }
    // Set an empty witness for the anchor input
    if let Some(anchor_index) = anchor_index {
        psbt.inputs[anchor_index].final_script_witness = Some(Witness::new())
    }
    Ok(())
}
