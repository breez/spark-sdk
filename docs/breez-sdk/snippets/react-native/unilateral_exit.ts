import type {
  BreezSdk,
  PrepareUnilateralExitResponse,
  UnilateralExitResponse
} from '@breeztech/breez-sdk-spark-react-native'
import {
  singleKeyCpfpSigner,
  CpfpFundingKind,
  CpfpInput,
  ExitLeafSelection,
  ConfirmationStatus,
  UnilateralExitVerdict_Tags
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
  // Buffer.buffer is a shared pool slab; slice to this key's own bytes.
  const signer = singleKeyCpfpSigner(
    secretKeyBytes.buffer.slice(
      secretKeyBytes.byteOffset,
      secretKeyBytes.byteOffset + secretKeyBytes.byteLength
    )
  )

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

const exampleCheckExit = async (sdk: BreezSdk, stored: UnilateralExitResponse) => {
  // ANCHOR: check-unilateral-exit
  const checked = await sdk.checkUnilateralExit({ exit: stored })

  // Store this one in place of the one you had.
  const exit = checked.exit

  switch (checked.verdict.tag) {
    case UnilateralExitVerdict_Tags.Valid:
      for (const tx of exit.transactions) {
        if (tx.dependenciesMet && tx.status !== ConfirmationStatus.Confirmed) {
          // Also wait out csvTimelockBlocks before broadcasting.
          console.log(`ready to broadcast: ${tx.txid}`)
        }
      }
      break
    case UnilateralExitVerdict_Tags.Done:
      console.log(`The exit finished: ${exit.recoverableValueSat} sats recovered`)
      break
    case UnilateralExitVerdict_Tags.Redo: {
      // Quote and build again, naming the same leaves. Pass exit.fundingInputs
      // back and the SDK follows them to whatever they have become.
      const { reason } = checked.verdict.inner
      console.log(`Build the exit again: ${reason}`)
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
class CustomCpfpSigner {
  signPsbt = async (psbtBytes: ArrayBuffer): Promise<ArrayBuffer> => {
    return await signPsbtWithYourKeys(psbtBytes)
  }
}

const signPsbtWithYourKeys = async (psbtBytes: ArrayBuffer): Promise<ArrayBuffer> => {
  return psbtBytes
}
// ANCHOR_END: custom-cpfp-signer
