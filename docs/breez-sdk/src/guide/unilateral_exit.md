# Unilateral Exit

A unilateral exit allows you to withdraw your funds from Spark to the Bitcoin blockchain without cooperation from the Spark operators. This is a safety mechanism — in normal operation, you should use the [cooperative withdrawal](send_payment.md#bitcoin) flow, which is cheaper and faster. A unilateral exit is your last resort when the operators are unresponsive or uncooperative.

<div class="warning">
<h4>Developer note</h4>
A unilateral exit is a complex, multi-step process that requires external Bitcoin funds to pay on-chain fees and may take hours or days to complete depending on timelock durations. Only use this when cooperative withdrawal is not possible.
</div>

## How it works

Spark stores your balance in a tree of pre-signed Bitcoin transactions. Each leaf in the tree represents a portion of your balance. To unilaterally exit, you broadcast the chain of transactions from the tree root down to the leaf you want to recover, followed by a refund transaction that sends the leaf's value to an address only you can spend from.

Each transaction in the chain has an **ephemeral anchor output** that requires a fee-bumping child transaction (CPFP — Child Pays For Parent) to incentivize miners. You provide an external UTXO to fund these CPFP transactions.

<img src="../images/unilateral_exit_tree.svg" alt="Unilateral exit transaction tree" style="display: block; margin: 2em auto; max-width: 700px;" />

The diagram above shows the structure for a single leaf. The node transactions form a path from the root to the leaf. The leaf transaction's output is spent by the refund transaction, which sends funds to your refund address. A final sweep transaction collects all refund outputs and sends them to your chosen destination. Every transaction in the chain (except the sweep) is paired with a CPFP fee-bump transaction that you must sign with your external UTXO key.

## Overview of the process

1. **List your leaves** to decide which ones to exit
2. **Prepare the unilateral exit** — the SDK builds all transactions and CPFP PSBTs
3. **Sign the CPFP PSBTs** with your external UTXO's private key
4. **Broadcast the packages** sequentially — each parent+child pair, waiting for confirmation between each
5. **Wait for timelocks** on the leaf and refund transactions
6. **Broadcast the sweep transaction** to collect your funds

## Step 1: List your leaves

Before starting a unilateral exit, inspect the leaves in your wallet to decide which ones to recover. Use {{#name list_leaves}} with a minimum value filter to exclude leaves that would cost more in fees than they are worth.

<div class="warning">
<h4>Choosing which leaves to exit</h4>

Each leaf requires broadcasting several transactions with fees. A leaf with a small value may cost more in fees than it holds. As a guideline:
<ul>
<li>The <b>absolute minimum</b> leaf value is 330 sats (the Bitcoin dust limit for P2TR outputs).</li>
<li>In practice, you should set a higher threshold. Each leaf requires broadcasting approximately 5–10 transaction packages, each costing fees proportional to the fee rate. At 2 sat/vbyte, each package costs roughly 550–636 sats, so a full exit of one leaf may cost 4,000–6,000 sats in CPFP fees alone, plus the sweep transaction fee.</li>
<li>A reasonable minimum is <b>10,000 sats</b> or more, depending on current fee conditions.</li>
</ul>
</div>

{{#tabs unilateral_exit:list-leaves}}

## Step 2: Prepare the unilateral exit

Call {{#name prepare_unilateral_exit}} with:
- The **leaf IDs** you want to exit
- One or more **external UTXOs** to fund the CPFP fee-bump transactions
- A **fee rate** in sats/vbyte
- A **destination address** where your funds will be swept to

The external UTXO must be a UTXO you control (P2WPKH or P2TR) with enough value to cover all CPFP fees. You will need the private key for this UTXO to sign the CPFP transactions.

{{#tabs unilateral_exit:prepare-unilateral-exit}}

The response contains:
- **{{#name leaves}}**: For each leaf, an ordered list of transaction/PSBT pairs to broadcast
- **{{#name sweep_tx_hex}}**: A fully signed transaction that sweeps all refund outputs to your destination

Each transaction pair contains:
- **{{#name parent_tx_hex}}**: The pre-signed Spark transaction (node TX, leaf TX, or refund TX)
- **{{#name child_psbt_hex}}**: An unsigned PSBT for the CPFP fee-bump transaction — **you must sign this**
- **{{#name csv_timelock_blocks}}**: If present, the number of blocks you must wait after the previous transaction confirms before this transaction can be included in a block

## Step 3: Sign the CPFP PSBTs

Each {{#name child_psbt_hex}} is an unsigned PSBT that spends from your external UTXO (and the parent transaction's ephemeral anchor output). You need to sign it with the private key of your external UTXO.

The PSBT contains two inputs:
1. **Your UTXO input** — requires your signature (P2WPKH or P2TR key-path spend)
2. **The ephemeral anchor input** — requires an empty witness (no signature needed)

Use any Bitcoin PSBT signing library to sign the PSBT. Here is an example using the `bitcoin` crate in Rust:

```rust
use bitcoin::{Psbt, PrivateKey, Witness, ecdsa::Signature, key::Secp256k1, sighash::SighashCache};

fn sign_cpfp_psbt(psbt: &mut Psbt, signing_key: &PrivateKey) {
    let secp = Secp256k1::new();
    let pubkey = signing_key.public_key(&secp);

    // Collect prevouts for sighash computation
    let prevouts: Vec<_> = psbt.inputs.iter()
        .map(|input| input.witness_utxo.clone().unwrap())
        .collect();

    let mut cache = SighashCache::new(&psbt.unsigned_tx);

    for (i, input) in psbt.inputs.iter_mut().enumerate() {
        let tx_out = input.witness_utxo.as_ref().unwrap();

        if tx_out.value.to_sat() == 0 {
            // Ephemeral anchor input — empty witness
            input.final_script_witness = Some(Witness::new());
        } else if tx_out.script_pubkey.is_p2wpkh() {
            // P2WPKH input — ECDSA signature
            let (msg, sighash_type) = psbt.sighash_ecdsa(i, &mut cache).unwrap();
            let sig = secp.sign_ecdsa(&msg, &signing_key.inner);
            let signature = Signature { signature: sig, sighash_type };
            let mut witness = Witness::new();
            witness.push(signature.to_vec());
            witness.push(pubkey.to_bytes());
            input.final_script_witness = Some(witness);
        } else if tx_out.script_pubkey.is_p2tr() {
            // P2TR input — Schnorr key-path spend
            let prevouts_ref = bitcoin::sighash::Prevouts::All(&prevouts);
            let sighash = cache.taproot_key_spend_signature_hash(
                i, &prevouts_ref, bitcoin::sighash::TapSighashType::Default,
            ).unwrap();
            let keypair = bitcoin::key::Keypair::from_secret_key(&secp, &signing_key.inner);
            let msg = bitcoin::secp256k1::Message::from_digest(sighash.to_byte_array());
            let schnorr_sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
            let tap_sig = bitcoin::taproot::Signature {
                signature: schnorr_sig,
                sighash_type: bitcoin::sighash::TapSighashType::Default,
            };
            let mut witness = Witness::new();
            witness.push(tap_sig.to_vec());
            input.final_script_witness = Some(witness);
        }
    }
}

// Sign each CPFP PSBT
for leaf in &response.leaves {
    for pair in &leaf.tx_cpfp_psbts {
        let mut psbt = Psbt::deserialize(&hex::decode(&pair.child_psbt_hex)?)?;
        sign_cpfp_psbt(&mut psbt, &your_private_key);
        let signed_tx = psbt.extract_tx()?;
        let signed_tx_hex = bitcoin::consensus::encode::serialize_hex(&signed_tx);
        // Store signed_tx_hex for broadcasting
    }
}
```

After signing, you can extract the final transaction from the PSBT using `extract_tx()`. You now have a pair of hex-encoded transactions for each step: the **parent transaction** and the **signed CPFP child transaction**.

## Step 4: Broadcast the packages

Bitcoin Core enforces a **1-parent-1-child (1p1c)** package relay policy. This means you must broadcast each parent+child pair as a **package** and wait for it to confirm before broadcasting the next one.

### Broadcasting order

For each leaf, the transaction pairs are returned in the order they must be broadcast:

| Order | Transaction | Timelock | Description |
|-------|-------------|----------|-------------|
| 1 | Root Node TX | None | First transaction in the chain, spends from the on-chain commitment |
| 2..N-2 | Intermediate Node TXs | None | Each spends from the previous node's output |
| N-1 | Leaf TX | **Yes** ({{#name csv_timelock_blocks}}) | Spends from the last node; has a relative timelock |
| N | Refund TX | **Yes** ({{#name csv_timelock_blocks}}) | Spends from the leaf TX; sends to your refund address |

### How to broadcast a package

Submit the parent transaction and the signed CPFP child transaction together as a package. If your Bitcoin node supports `submitpackage` (Bitcoin Core 28.0+), use it:

```
bitcoin-cli submitpackage '["<parent_tx_hex>", "<signed_cpfp_child_tx_hex>"]'
```

Many block explorers also support package submission by accepting a comma-separated pair of raw transactions.

### Handling timelocks

When a transaction has a {{#name csv_timelock_blocks}} value, you **cannot broadcast it** until the specified number of blocks have been mined after the previous transaction confirms. For example, if the leaf TX has {{#name csv_timelock_blocks}}: 1700, you must wait 1,700 blocks (~12 days) after the last node TX confirms before broadcasting the leaf TX package.

<div class="warning">
<h4>Developer note</h4>
The timelock is a <b>relative</b> lock (BIP68 CSV). It counts blocks from the confirmation of the output being spent, not from any absolute block height. Your application should monitor the blockchain and broadcast the next package as soon as the timelock expires.
</div>

### Step-by-step broadcasting procedure

```
For each leaf in the response:
    For each (parent_tx, signed_cpfp_child_tx) pair:
        1. If csv_timelock_blocks is set:
             Wait until the previous transaction has at least
             csv_timelock_blocks confirmations.
        2. Submit the package: [parent_tx_hex, signed_cpfp_child_tx_hex]
        3. Wait for the package to confirm (at least 1 confirmation).
```

If you are exiting multiple leaves, some node transactions may be shared between leaves (they share the same path from the root). The SDK deduplicates these automatically — each unique node transaction appears only once in the first leaf that uses it. Subsequent leaves start from where the shared path diverges.

## Step 5: Broadcast the sweep transaction

After **all** refund transactions for all leaves have confirmed, broadcast the sweep transaction ({{#name sweep_tx_hex}}). This is a standard Bitcoin transaction (not a package) that spends from all refund outputs and sends the total value (minus fees) to your destination address.

```
bitcoin-cli sendrawtransaction "<sweep_tx_hex>"
```

<div class="warning">
<h4>Developer note</h4>
The sweep transaction can only be broadcast after every refund transaction has confirmed. If you are exiting multiple leaves, all their refund transactions must confirm before the sweep can be broadcast.
</div>

## Fee considerations

A unilateral exit involves two types of fees:

1. **CPFP fees**: Paid from your external UTXO to fee-bump each transaction in the chain. The total CPFP cost depends on the tree depth and the fee rate. A typical single-leaf exit has 5–8 transaction pairs, each costing roughly `fee_rate * 275–318` sats.

2. **Sweep transaction fee**: Deducted from the refund output values. This fee is calculated at the same fee rate you specify in the prepare request.

### CPFP change output

Each CPFP fee-bump transaction has a single output that sends the remaining value (your UTXO value minus the fee) back to the **same address** as the original external UTXO. This change output is automatically used as the input for the next CPFP transaction in the chain. After the last CPFP transaction confirms, the remaining change sits in an output at that same address and is yours to spend.

Ensure the external UTXO has enough value to cover all CPFP fees for all leaves you are exiting.

## Troubleshooting

| Problem | Cause | Solution |
|---------|-------|----------|
| "min relay fee not met" | CPFP fee too low for the package | Increase the {{#name fee_rate}} parameter |
| "mandatory-script-verify-flag-failed" | PSBT not signed correctly | Ensure you are finalizing the PSBT inputs (setting `final_script_witness`) before extracting |
| "non-BIP68-final" | Timelock has not expired | Wait for the required number of confirmations on the previous transaction |
| Transaction not relayed | Parent+child not submitted as package | Use `submitpackage` or a block explorer's package submission |
| Sweep transaction rejected | Not all refund TXs confirmed yet | Wait for all refund transactions to confirm before broadcasting the sweep |
