import {
  type BreezSdk,
  type ListUnclaimedDepositsRequest,
  type ClaimDepositRequest,
  type RefundDepositRequest,
  type Fee
} from '@breeztech/breez-sdk-spark'

const listUnclaimedDeposits = async (sdk: BreezSdk) => {
  // ANCHOR: list-unclaimed-deposits
  const request: ListUnclaimedDepositsRequest = {}
  const response = await sdk.listUnclaimedDeposits(request)

  for (const deposit of response.deposits) {
    console.log(`Unclaimed deposit: ${deposit.txid}:${deposit.vout}`)
    console.log(`Amount: ${deposit.amountSats} sats`)

    if (deposit.claimError != null) {
      switch (deposit.claimError.type) {
        case 'depositClaimFeeExceeded': {
          let maxFeeStr = 'none'
          if (deposit.claimError.maxFee != null) {
            maxFeeStr = deposit.claimError.maxFee.type === 'fixed'
              ? `${deposit.claimError.maxFee.amount} sats`
              : `${deposit.claimError.maxFee.satPerVbyte} sat/vB`
          }
          console.log(
            `Max claim fee exceeded. Max: ${maxFeeStr}, Actual: ${deposit.claimError.actualFee} sats`
          )
          break
        }
        case 'missingUtxo':
          console.log('UTXO not found when claiming deposit')
          break
        case 'generic':
          console.log(`Claim failed: ${deposit.claimError.message}`)
          break
      }
    }
  }
  // ANCHOR_END: list-unclaimed-deposits
}

const claimDeposit = async (sdk: BreezSdk) => {
  // ANCHOR: claim-deposit
  const txid = 'your_deposit_txid'
  const vout = 0

  // Set a higher max fee to retry claiming
  const maxFee: Fee = {
    type: 'fixed',
    amount: 5_000
  }

  const request: ClaimDepositRequest = {
    txid,
    vout,
    maxFee
  }

  const response = await sdk.claimDeposit(request)
  console.log('Deposit claimed successfully. Payment:', response.payment)
  // ANCHOR_END: claim-deposit
}

const refundDeposit = async (sdk: BreezSdk) => {
  // ANCHOR: refund-deposit
  const txid = 'your_deposit_txid'
  const vout = 0
  const destinationAddress = 'bc1qexample...' // Your Bitcoin address

  // Set the fee for the refund transaction using a rate
  const fee: Fee = { type: 'rate', satPerVbyte: 5 }
  // or using a fixed amount
  // const fee: Fee = { type: 'fixed', amount: 500 }

  const request: RefundDepositRequest = {
    txid,
    vout,
    destinationAddress,
    fee
  }

  const response = await sdk.refundDeposit(request)
  console.log('Refund transaction created:')
  console.log('Transaction ID:', response.txId)
  console.log('Transaction hex:', response.txHex)
  // ANCHOR_END: refund-deposit
}

const recommendedFees = async (sdk: BreezSdk) => {
  // ANCHOR: recommended-fees
  const response = await sdk.recommendedFees()
  console.log('Fastest fee:', response.fastestFee)
  console.log('Half-hour fee:', response.halfHourFee)
  console.log('Hour fee:', response.hourFee)
  console.log('Economy fee:', response.economyFee)
  console.log('Minimum fee:', response.minimumFee)
  // ANCHOR_END: recommended-fees
}
