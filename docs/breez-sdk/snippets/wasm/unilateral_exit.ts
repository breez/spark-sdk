import type {
  BreezSdk,
  CpfpSigner,
  Leaf,
  PrepareUnilateralExitResponse
} from '@breeztech/breez-sdk-spark'

const exampleListLeavesForExit = async (sdk: BreezSdk): Promise<Leaf[]> => {
  // ANCHOR: list-leaves
  const response = await sdk.listLeaves({
    minValueSats: 10_000
  })

  for (const leaf of response.leaves) {
    console.log(`Leaf ${leaf.id}: ${leaf.value} sats`)
  }
  // ANCHOR_END: list-leaves
  return response.leaves
}

const examplePrepareExit = async (sdk: BreezSdk): Promise<PrepareUnilateralExitResponse> => {
  // ANCHOR: prepare-unilateral-exit
  const leafIds = ['leaf-id-1', 'leaf-id-2']
  const signer: CpfpSigner = {
    signPsbt: async (_psbtBytes: Uint8Array): Promise<Uint8Array> => {
      // Sign the PSBT with your UTXO key and return the signed bytes
      throw new Error('not implemented')
    }
  }
  const response = await sdk.prepareUnilateralExit(
    {
      feeRate: 2,
      leafIds,
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

  for (const leaf of response.leaves) {
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
