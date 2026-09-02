import type {
  BreezSdk,
  CpfpSigner,
  PrepareUnilateralExitResponse,
  UnilateralExitResponse
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

const exampleCheckExit = async (sdk: BreezSdk, stored: UnilateralExitResponse) => {
  // ANCHOR: check-unilateral-exit
  const checked = await sdk.checkUnilateralExit({ exit: stored })

  // Store this one in place of the one you had.
  const exit = checked.exit

  switch (checked.verdict.type) {
    case 'valid': {
      for (const tx of exit.transactions) {
        if (tx.dependenciesMet && tx.status !== 'confirmed') {
          // Also wait out csvTimelockBlocks before broadcasting.
          console.log(`ready to broadcast: ${tx.txid}`)
        }
      }
      break
    }
    case 'done': {
      console.log(`The exit finished: ${exit.recoverableValueSat} sats recovered`)
      break
    }
    case 'redo': {
      // Quote and build again, naming the same leaves. Pass exit.fundingInputs
      // back and the SDK follows them to whatever they have become.
      console.log(`Build the exit again: ${checked.verdict.reason}`)
      break
    }
  }
  // ANCHOR_END: check-unilateral-exit
}

const exampleExportExitState = async (sdk: BreezSdk): Promise<string> => {
  // ANCHOR: export-unilateral-exit-state
  const exported = await sdk.exportUnilateralExitState()

  // Keep the state somewhere the wallet's own storage cannot take with it.
  console.log(`Exit state is ${exported.exitState.length} bytes`)
  // ANCHOR_END: export-unilateral-exit-state
  return exported.exitState
}

const exampleImportExitState = async (sdk: BreezSdk, exitState: string) => {
  // ANCHOR: import-unilateral-exit-state
  const imported = await sdk.importUnilateralExitState({ exitState })

  console.log(`Imported ${imported.importedLeaves} leaves, skipped ${imported.skippedForeignLeaves}`)
  // ANCHOR_END: import-unilateral-exit-state
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
