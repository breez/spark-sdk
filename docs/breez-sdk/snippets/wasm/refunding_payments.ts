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
    
    if (deposit.claimError) {
      switch (deposit.claimError.type) {
        case 'depositClaimFeeExceeded':
          console.log(`Claim failed: Fee exceeded. Max: ${deposit.claimError.maxFee}, Actual: ${deposit.claimError.actualFee}`)
          break
        case 'missingUtxo':
          console.log('Claim failed: UTXO not found')
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
  
  // Set the fee for the refund transaction
  const fee: Fee = { type: 'fixed', amount: 500 }
  
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
