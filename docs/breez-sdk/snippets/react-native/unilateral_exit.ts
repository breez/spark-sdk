import type {
  BreezSdk,
  PrepareUnilateralExitResponse
} from '@breeztech/breez-sdk-spark-react-native'
import {
  SingleKeySigner,
  UnilateralExitCpfpInput_Tags
} from '@breeztech/breez-sdk-spark-react-native'

const examplePrepareExit = async (sdk: BreezSdk): Promise<PrepareUnilateralExitResponse> => {
  // ANCHOR: prepare-unilateral-exit
  // Create a signer from your UTXO private key (32-byte secret key)
  const secretKeyBytes = Buffer.from('your-secret-key-hex', 'hex')
  const signer = new SingleKeySigner({ secretKeyBytes })

  const response = await sdk.prepareUnilateralExit({
    feeRate: BigInt(2),
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

  // The SDK automatically selects which leaves are profitable to exit.
  for (const leaf of response.selectedLeaves) {
    console.log(`Leaf ${leaf.id}: ${leaf.value} sats (exit cost: ~${leaf.estimatedCost} sats)`)
  }

  // The response contains signed transactions ready to broadcast:
  // - response.transactions: parent/child transaction pairs per leaf
  // - response.sweepTxHex: signed sweep transaction for the final step
  // Change from CPFP fee-bumping always goes back to the first input's address.
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
