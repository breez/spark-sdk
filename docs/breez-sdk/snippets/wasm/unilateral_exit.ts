import type {
  BreezSdk,
  CpfpSigner,
  PrepareUnilateralExitResponse
} from '@breeztech/breez-sdk-spark'
import { singleKeyCpfpSigner } from '@breeztech/breez-sdk-spark'

const exampleQuoteExit = async (sdk: BreezSdk): Promise<PrepareUnilateralExitResponse> => {
  // ANCHOR: prepare-unilateral-exit
  const quote = await sdk.prepareUnilateralExit({
    feeRateSatPerVbyte: 2,
    fundingKind: { type: 'p2wpkh' },
    destination: 'bc1q...your-destination-address',
    selection: { type: 'auto' }
  })

  console.log(`Recovering ${quote.recoverableValueSat} sats for ${quote.totalFeeSat} sats in fees`)
  console.log(`Fund a single UTXO of at least ${quote.singleUtxoFundingSat} sats`)
  // ANCHOR_END: prepare-unilateral-exit
  return quote
}

const exampleBuildExit = async (sdk: BreezSdk, quote: PrepareUnilateralExitResponse) => {
  // ANCHOR: unilateral-exit
  const secretKeyBytes = Buffer.from('your-secret-key-hex', 'hex')
  const signer = singleKeyCpfpSigner(secretKeyBytes)

  const response = await sdk.unilateralExit(
    {
      prepared: quote,
      fundingInputs: [{
        type: 'p2wpkh',
        txid: 'your-utxo-txid',
        vout: 0,
        value: 50_000,
        pubkey: 'your-compressed-pubkey-hex'
      }]
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
class CustomCpfpSigner implements CpfpSigner {
  async signPsbt (psbtBytes: Uint8Array): Promise<Uint8Array> {
    return await signPsbtWithYourKeys(psbtBytes)
  }
}

const signPsbtWithYourKeys = async (psbtBytes: Uint8Array): Promise<Uint8Array> => {
  return psbtBytes
}
// ANCHOR_END: custom-cpfp-signer
