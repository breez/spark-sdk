import type {
  BreezSdk,
  PrepareRecoverFundsResponse,
  RecoverFundsResponse
} from '@breeztech/breez-sdk-spark-react-native'
import {
  singleKeyCpfpSigner,
  CooperativeRecoveryError_Tags,
  CpfpFundingKind,
  CpfpInput,
  ExitLeafSelection,
  ExitTransactionStatus_Tags,
  RecoveryMethod,
  RecoveryRedoReason,
  RecoveryVerdict_Tags
} from '@breeztech/breez-sdk-spark-react-native'

const exampleFetchRecoverableFunds = async (sdk: BreezSdk) => {
  // ANCHOR: recoverable-funds
  const info = await sdk.getInfo({ ensureSynced: false })

  if (info.recoverableFundsSats > 0) {
    console.log(`${info.recoverableFundsSats} sats can be recovered on-chain`)
  }
  // ANCHOR_END: recoverable-funds
}

const examplePrepareRecovery = async (sdk: BreezSdk): Promise<PrepareRecoverFundsResponse> => {
  // ANCHOR: prepare-recover-funds
  const quote = await sdk.prepareRecoverFunds({
    feeRateSatPerVbyte: BigInt(2),
    fundingKind: new CpfpFundingKind.P2wpkh(),
    destination: 'bc1q...your-destination-address',
    selection: new ExitLeafSelection.RecoverableOnly()
  })

  if (quote.leaves.length === 0) {
    console.log('Nothing to recover')
    return quote
  }
  for (const leaf of quote.leaves) {
    console.log(`${leaf.leafId}: ${leaf.valueSats} sats, ${RecoveryMethod[leaf.method]}`)
  }
  console.log(
    `Recovering ${quote.recoverableValueSats} sats for ${quote.totalFeeSats} sats in fees`
  )
  if (quote.funding != null) {
    console.log(`Fund one UTXO of at least ${quote.funding.singleUtxoSats} sats`)
  }
  // ANCHOR_END: prepare-recover-funds

  return quote
}

const exampleRecoverCooperatively = async (sdk: BreezSdk, quote: PrepareRecoverFundsResponse) => {
  // ANCHOR: recover-cooperatively
  // A quote with funding holds a unilateral exit: prepare the
  // cooperative leaves alone to recover them without it.
  let prepared = quote
  if (quote.funding != null) {
    const leafIds = quote.leaves
      .filter((leaf) => leaf.method === RecoveryMethod.Cooperative)
      .map((leaf) => leaf.leafId)
    if (leafIds.length === 0) {
      return
    }
    prepared = await sdk.prepareRecoverFunds({
      feeRateSatPerVbyte: quote.feeRateSatPerVbyte,
      fundingKind: undefined,
      destination: quote.destination,
      selection: new ExitLeafSelection.Specific({ leafIds })
    })
  }
  const response = await sdk.recoverFunds(
    {
      prepared,
      fundingInputs: []
    },
    undefined
  )

  // Keep the whole response: checkRecoverFunds follows the recovery from it.
  for (const tx of response.transactions) {
    console.log(`Broadcast ${tx.txid}: ${tx.txHex}`)
  }
  for (const { leafId, error } of response.failed) {
    const reason =
      error.tag === CooperativeRecoveryError_Tags.ReplacementFeeTooLow
        ? `replacing a recovery on the network takes at least ${error.inner.requiredFeeSats} sats`
        : error.inner.message
    console.log(`Leaf ${leafId} was not recovered: ${reason}`)
  }
  // ANCHOR_END: recover-cooperatively
}

const exampleRecoverWithFunding = async (
  sdk: BreezSdk,
  quote: PrepareRecoverFundsResponse
): Promise<RecoverFundsResponse> => {
  // ANCHOR: recover-funds
  const secretKeyBytes = Buffer.from('your-secret-key-hex', 'hex')
  // Buffer.buffer is a shared pool slab; slice to this key's own bytes.
  const signer = singleKeyCpfpSigner(
    secretKeyBytes.buffer.slice(
      secretKeyBytes.byteOffset,
      secretKeyBytes.byteOffset + secretKeyBytes.byteLength
    )
  )

  const response = await sdk.recoverFunds(
    {
      prepared: quote,
      fundingInputs: [
        new CpfpInput.P2wpkh({
          txid: 'your-utxo-txid',
          vout: 0,
          valueSats: BigInt(50_000),
          pubkey: 'your-compressed-pubkey-hex'
        })
      ]
    },
    signer
  )

  // Keep the whole response: checkRecoverFunds follows the recovery from it.
  for (const tx of response.transactions) {
    if (tx.csvTimelockBlocks != null) {
      console.log(`${tx.txid}: wait ${tx.csvTimelockBlocks} blocks after its parents confirm`)
    }
  }
  // ANCHOR_END: recover-funds

  return response
}

const exampleCheckRecovery = async (sdk: BreezSdk, stored: RecoverFundsResponse) => {
  // ANCHOR: check-recover-funds
  const checked = await sdk.checkRecoverFunds({ recovery: stored })

  // Store this one in place of the one you had.
  const recovery = checked.recovery

  switch (checked.verdict.tag) {
    case RecoveryVerdict_Tags.Valid:
      for (const tx of recovery.transactions) {
        if (tx.status.tag === ExitTransactionStatus_Tags.Ready) {
          console.log(`ready to broadcast: ${tx.txid}`)
        }
      }
      break
    case RecoveryVerdict_Tags.Done:
      console.log('Every transaction confirmed: the recovery is done')
      break
    case RecoveryVerdict_Tags.Redo: {
      // Prepare and build again, naming the same leaves. Pass
      // recovery.fundingInputs back and the SDK follows them to whatever
      // they have become.
      const { reason } = checked.verdict.inner
      console.log(`Build the recovery again: ${RecoveryRedoReason[reason]}`)
      break
    }
  }
  // ANCHOR_END: check-recover-funds
}

const exampleBackUpExitState = async (sdk: BreezSdk): Promise<string> => {
  // ANCHOR: export-exit-state
  const exported = await sdk.exportUnilateralExitState()

  // Keep the state somewhere the wallet's own storage cannot take with it.
  console.log(`Exit state is ${exported.exitState.length} bytes`)
  // ANCHOR_END: export-exit-state

  return exported.exitState
}

const exampleRestoreExitState = async (sdk: BreezSdk, exitState: string) => {
  // ANCHOR: import-exit-state
  const imported = await sdk.importUnilateralExitState({ exitState })

  console.log(
    `Imported ${imported.importedLeaves} leaves, skipped ${imported.skippedForeignLeaves}`
  )
  // ANCHOR_END: import-exit-state
}

const exampleCollectExitData = async (sdk: BreezSdk) => {
  // ANCHOR: sync-exit-data
  // With automatic collection off, an explicit sync is what collects the data
  // a unilateral exit needs, and it waits for the collection to finish. Needs
  // the Spark operators reachable, so run it on a schedule rather than at the
  // moment an exit is needed.
  await sdk.syncWallet({})
  // ANCHOR_END: sync-exit-data
}

// ANCHOR: custom-cpfp-signer
class MyFundingSigner {
  signPsbt = async (psbtBytes: ArrayBuffer): Promise<ArrayBuffer> => {
    return await signWithFundingKeys(psbtBytes)
  }
}

const signWithFundingKeys = async (psbtBytes: ArrayBuffer): Promise<ArrayBuffer> => {
  return psbtBytes
}
// ANCHOR_END: custom-cpfp-signer
