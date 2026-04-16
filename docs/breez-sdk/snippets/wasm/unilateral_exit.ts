import type {
  BreezSdk,
  CpfpSigner,
  PrepareUnilateralExitResponse
} from '@breeztech/breez-sdk-spark'

const examplePrepareExit = async (sdk: BreezSdk): Promise<PrepareUnilateralExitResponse> => {
  // ANCHOR: prepare-unilateral-exit
  const signer: CpfpSigner = {
    signPsbt: async (_psbtBytes: Uint8Array): Promise<Uint8Array> => {
      // Sign the PSBT with your UTXO key and return the signed bytes
      throw new Error('not implemented')
    }
  }
  const response = await sdk.prepareUnilateralExit(
    {
      feeRate: 2,
      inputs: [{
        type: 'p2wpkh',
        txid: 'your-utxo-txid',
        vout: 0,
        value: BigInt(50_000),
        pubkey: 'your-compressed-pubkey-hex'
      }],
      destination: 'bc1q...your-destination-address'
    },
    signer
  )

  // The SDK automatically selects which leaves are profitable to exit.
  for (const leaf of response.leaves) {
    console.log(`Leaf ${leaf.leafId}: ${leaf.value} sats (exit cost: ~${leaf.estimatedCost} sats)`)
    for (const tx of leaf.transactions) {
      if (tx.csvTimelockBlocks != null) {
        console.log(`Timelock: wait ${tx.csvTimelockBlocks} blocks`)
      }
      // tx.txHex: pre-signed Spark transaction
      // tx.cpfpTxHex: signed CPFP transaction — broadcast alongside parent
    }
  }

  if (response.unverifiedNodeIds.length > 0) {
    console.log(`Warning: could not verify confirmation status for ${response.unverifiedNodeIds.length} nodes`)
  }
  // ANCHOR_END: prepare-unilateral-exit
  return response
}
