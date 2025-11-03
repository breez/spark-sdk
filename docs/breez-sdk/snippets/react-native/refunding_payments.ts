import {
  type BreezSdk,
  type ListUnclaimedDepositsRequest,
  type ClaimDepositRequest,
  type RefundDepositRequest,
  DepositClaimError,
  Fee
} from '@breeztech/breez-sdk-spark-react-native'

const listUnclaimedDeposits = async (sdk: BreezSdk) => {
  // ANCHOR: list-unclaimed-deposits
  const request: ListUnclaimedDepositsRequest = {}
  const response = await sdk.listUnclaimedDeposits(request)

  for (const deposit of response.deposits) {
    console.log(`Unclaimed deposit: ${deposit.txid}:${deposit.vout}`)
    console.log(`Amount: ${deposit.amountSats} sats`)

    if (deposit.claimError != null) {
      if (deposit.claimError instanceof DepositClaimError.DepositClaimFeeExceeded) {
        let maxFeeStr = 'none'
        if (deposit.claimError.inner.maxFee instanceof Fee.Fixed) {
          maxFeeStr = `${deposit.claimError.inner.maxFee.inner.amount} sats`
        } else if (deposit.claimError.inner.maxFee instanceof Fee.Rate) {
          maxFeeStr = `${deposit.claimError.inner.maxFee.inner.satPerVbyte} sat/vB`
        }
        console.log(
          `Max claim fee exceeded. Max: ${maxFeeStr}, Actual: ${deposit.claimError.inner.actualFee} sats`
        )
      } else if (deposit.claimError instanceof DepositClaimError.MissingUtxo) {
        console.log('UTXO not found when claiming deposit')
      } else if (deposit.claimError instanceof DepositClaimError.Generic) {
        console.log(`Claim failed: ${deposit.claimError.inner.message}`)
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
  const maxFee = new Fee.Fixed({ amount: BigInt(5000) })

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

  // Set the fee for the refund transaction
  const fee = new Fee.Fixed({ amount: BigInt(500) })

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
