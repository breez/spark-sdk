import type {
  BreezSdk,
  PrepareUnilateralExitResponse
} from '@breeztech/breez-sdk-spark-react-native'
import {
  singleKeyCpfpSigner,
  CpfpFundingKind,
  CpfpInput,
  ExitLeafSelection
} from '@breeztech/breez-sdk-spark-react-native'

const exampleQuoteExit = async (sdk: BreezSdk): Promise<PrepareUnilateralExitResponse> => {
  // ANCHOR: prepare-unilateral-exit
  const quote = await sdk.prepareUnilateralExit({
    feeRateSatPerVbyte: BigInt(2),
    fundingKind: new CpfpFundingKind.P2wpkh(),
    destination: 'bc1q...your-destination-address',
    selection: new ExitLeafSelection.Auto()
  })

  console.log(`Recovering ${quote.recoverableValueSat} sats for ${quote.totalFeeSat} sats in fees`)
  console.log(`Fund a single UTXO of at least ${quote.singleUtxoFundingSat} sats`)
  // ANCHOR_END: prepare-unilateral-exit
  return quote
}

const exampleBuildExit = async (sdk: BreezSdk, quote: PrepareUnilateralExitResponse) => {
  // ANCHOR: unilateral-exit
  const secretKeyBytes = Buffer.from('your-secret-key-hex', 'hex')
  const signer = singleKeyCpfpSigner(secretKeyBytes.buffer)

  const response = await sdk.unilateralExit(
    {
      prepared: quote,
      fundingInputs: [
        new CpfpInput.P2wpkh({
          txid: 'your-utxo-txid',
          vout: 0,
          value: BigInt(50_000),
          pubkey: 'your-compressed-pubkey-hex'
        })
      ]
    },
    signer
  )

  for (const tx of response.transactions) {
    if (tx.csvTimelockBlocks != null) {
      console.log(`${tx.txid}: wait ${tx.csvTimelockBlocks} blocks after its parents confirm`)
    }
  }
  // ANCHOR_END: unilateral-exit
}

// ANCHOR: custom-cpfp-signer
class CustomCpfpSigner {
  signPsbt = async (psbtBytes: ArrayBuffer): Promise<ArrayBuffer> => {
    return await signPsbtWithYourKeys(psbtBytes)
  }
}

const signPsbtWithYourKeys = async (psbtBytes: ArrayBuffer): Promise<ArrayBuffer> => {
  return psbtBytes
}
// ANCHOR_END: custom-cpfp-signer
