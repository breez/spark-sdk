import type {
  BreezSdk,
  PrepareUnilateralExitResponse
} from '@breeztech/breez-sdk-spark-react-native'
import {
  SingleKeySigner,
  UnilateralExitCpfpInput
} from '@breeztech/breez-sdk-spark-react-native'

const examplePrepareExit = async (sdk: BreezSdk): Promise<PrepareUnilateralExitResponse> => {
  // ANCHOR: prepare-unilateral-exit
  // Create a signer from your UTXO private key (32-byte secret key)
  const secretKeyBytes = Buffer.from('your-secret-key-hex', 'hex')
  const signer = new SingleKeySigner(secretKeyBytes)

  const response = await sdk.prepareUnilateralExit({
    feeRate: BigInt(2),
    inputs: [
      new UnilateralExitCpfpInput.P2wpkh({
        txid: 'your-utxo-txid',
        vout: 0,
        value: BigInt(50_000),
        pubkey: 'your-compressed-pubkey-hex'
      })
    ],
    destination: 'bc1q...your-destination-address'
  }, signer)

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
