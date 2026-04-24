import type {
  BreezSdk,
  Leaf,
  PrepareUnilateralExitResponse
} from '@breeztech/breez-sdk-spark-react-native'
import {
  SingleKeySigner,
  UnilateralExitCpfpInput_Tags
} from '@breeztech/breez-sdk-spark-react-native'

const exampleListLeavesForExit = async (sdk: BreezSdk): Promise<Leaf[]> => {
  // ANCHOR: list-leaves
  const response = await sdk.listLeaves({
    minValueSats: BigInt(10_000)
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

  // Create a signer from your UTXO private key (32-byte secret key)
  const secretKeyBytes = Buffer.from('your-secret-key-hex', 'hex')
  const signer = new SingleKeySigner({ secretKeyBytes })

  const response = await sdk.prepareUnilateralExit({
    feeRate: BigInt(2),
    leafIds,
    inputs: [{
      tag: UnilateralExitCpfpInput_Tags.P2wpkh,
      inner: {
        txid: 'your-utxo-txid',
        vout: 0,
        value: BigInt(50_000),
        pubkey: 'your-compressed-pubkey-hex'
      }
    }],
    destination: 'bc1q...your-destination-address'
  }, signer)

  // The response contains signed transactions ready to broadcast:
  // - response.leaves: parent/child transaction pairs
  // - response.sweepTxHex: signed sweep transaction for the final step
  // Change from CPFP fee-bumping always goes back to the first input's address.
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
