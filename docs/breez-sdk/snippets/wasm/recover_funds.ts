import type {
  BreezSdk,
  CpfpSigner,
  PrepareRecoverFundsResponse,
  RecoverFundsResponse
} from '@breeztech/breez-sdk-spark'
import { singleKeyCpfpSigner } from '@breeztech/breez-sdk-spark'

const exampleFetchRecoverableFunds = async (sdk: BreezSdk) => {
  // ANCHOR: recoverable-funds
  const info = await sdk.getInfo({ ensureSynced: false })

  if (info.recoverableFundsSats > 0) {
    console.log(`${info.recoverableFundsSats} sats can be recovered on-chain`)
  }
  // ANCHOR_END: recoverable-funds
}

const exampleQuoteRecovery = async (sdk: BreezSdk): Promise<PrepareRecoverFundsResponse> => {
  // ANCHOR: prepare-recover-funds
  const quote = await sdk.prepareRecoverFunds({
    feeRateSatPerVbyte: 2,
    fundingKind: { type: 'p2wpkh' },
    destination: 'bc1q...your-destination-address',
    selection: { type: 'recoverableOnly' }
  })

  if (quote.leaves.length === 0) {
    console.log('Nothing to recover')
    return quote
  }
  for (const leaf of quote.leaves) {
    console.log(`${leaf.leafId}: ${leaf.valueSats} sats, ${leaf.method}`)
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
  // A quote that asks for funding holds a unilateral exit: quote the
  // cooperative leaves alone to recover them without it.
  let prepared = quote
  if (quote.funding != null) {
    const leafIds = quote.leaves
      .filter((leaf) => leaf.method === 'cooperative')
      .map((leaf) => leaf.leafId)
    if (leafIds.length === 0) {
      return
    }
    prepared = await sdk.prepareRecoverFunds({
      feeRateSatPerVbyte: quote.feeRateSatPerVbyte,
      destination: quote.destination,
      selection: { type: 'specific', leafIds }
    })
  }
  const response = await sdk.recoverFunds({ prepared }, undefined)

  // Keep the whole response: checkRecoverFunds follows the recovery from it.
  for (const tx of response.transactions) {
    console.log(`Broadcast ${tx.txid}: ${tx.txHex}`)
  }
  for (const failure of response.failed) {
    console.log(`Leaf ${failure.leafId} was not recovered: ${JSON.stringify(failure.error)}`)
  }
  // ANCHOR_END: recover-cooperatively
}

const exampleRecoverWithFunding = async (
  sdk: BreezSdk,
  quote: PrepareRecoverFundsResponse
): Promise<RecoverFundsResponse> => {
  // ANCHOR: recover-funds
  const secretKeyBytes = Buffer.from('your-secret-key-hex', 'hex')
  const signer = singleKeyCpfpSigner(secretKeyBytes)

  const response = await sdk.recoverFunds(
    {
      prepared: quote,
      fundingInputs: [{
        type: 'p2wpkh',
        txid: 'your-utxo-txid',
        vout: 0,
        valueSats: 50_000,
        pubkey: 'your-compressed-pubkey-hex'
      }]
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

  switch (checked.verdict.type) {
    case 'valid': {
      for (const tx of recovery.transactions) {
        if (tx.status.type === 'ready') {
          console.log(`ready to broadcast: ${tx.txid}`)
        }
      }
      break
    }
    case 'done': {
      console.log('Every transaction confirmed: the recovery is done')
      break
    }
    case 'redo': {
      // Quote and build again, naming the same leaves. Pass recovery.fundingInputs
      // back and the SDK follows them to whatever they have become.
      console.log(`Build the recovery again: ${checked.verdict.reason}`)
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
class MyFundingSigner implements CpfpSigner {
  async signPsbt (psbtBytes: Uint8Array): Promise<Uint8Array> {
    return await signWithFundingKeys(psbtBytes)
  }
}

const signWithFundingKeys = async (psbtBytes: Uint8Array): Promise<Uint8Array> => {
  return psbtBytes
}
// ANCHOR_END: custom-cpfp-signer
