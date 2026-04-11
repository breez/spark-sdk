import type {
  BreezSdk,
  Leaf,
  PrepareUnilateralExitResponse
} from '@breeztech/breez-sdk-spark-react-native'
import { UnilateralExitCpfpUtxoType } from '@breeztech/breez-sdk-spark-react-native'

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

  const response = await sdk.prepareUnilateralExit({
    feeRate: BigInt(2),
    leafIds,
    utxos: [{
      txid: 'your-utxo-txid',
      vout: 0,
      value: BigInt(50_000),
      pubkey: 'your-compressed-pubkey-hex',
      utxoType: UnilateralExitCpfpUtxoType.P2wpkh
    }],
    destination: 'bc1q...your-destination-address'
  })

  // The response contains:
  // - response.leaves: transaction/PSBT pairs to sign and broadcast
  // - response.sweepTxHex: signed sweep transaction for the final step
  for (const leaf of response.leaves) {
    for (const pair of leaf.txCpfpPsbts) {
      if (pair.csvTimelockBlocks != null) {
        console.log(`Timelock: wait ${pair.csvTimelockBlocks} blocks`)
      }
      // pair.parentTxHex: pre-signed Spark transaction
      // pair.childPsbtHex: unsigned CPFP PSBT — sign with your UTXO key
    }
  }
  // ANCHOR_END: prepare-unilateral-exit
  return response
}
