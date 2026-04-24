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
  for (const leaf of response.selectedLeaves) {
    console.log(`Leaf ${leaf.id}: ${leaf.value} sats (exit cost: ~${leaf.estimatedCost} sats)`)
  }

  for (const leaf of response.transactions) {
    for (const pair of leaf.txCpfpPairs) {
      if (pair.csvTimelockBlocks != null) {
        console.log(`Timelock: wait ${pair.csvTimelockBlocks} blocks`)
      }
      // pair.parentTxHex: pre-signed Spark transaction
      // pair.childTxHex: signed CPFP transaction — broadcast alongside parent
    }
  }
  // ANCHOR_END: prepare-unilateral-exit
  return response
}
