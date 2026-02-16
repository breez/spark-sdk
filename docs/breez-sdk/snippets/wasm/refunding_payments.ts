import {
  type BreezClient,
  type ClaimDepositRequest,
  type RefundDepositRequest,
  type Fee,
  type DepositInfo,
  defaultConfig,
  Network,
  type MaxFee
} from '@breeztech/breez-sdk-spark'

const listUnclaimedDeposits = async (client: BreezClient) => {
  // ANCHOR: list-unclaimed-deposits
  const deposits = await client.deposits.listUnclaimed()

  for (const deposit of deposits) {
    console.log(`Unclaimed deposit: ${deposit.txid}:${deposit.vout}`)
    console.log(`Amount: ${deposit.amountSats} sats`)

    if (deposit.claimError != null) {
      switch (deposit.claimError.type) {
        case 'maxDepositClaimFeeExceeded': {
          let maxFeeStr = 'none'
          if (deposit.claimError.maxFee != null) {
            if (deposit.claimError.maxFee.type === 'fixed') {
              maxFeeStr = `${deposit.claimError.maxFee.amount} sats`
            } else if (deposit.claimError.maxFee.type === 'rate') {
              maxFeeStr = `${deposit.claimError.maxFee.satPerVbyte} sats/vByte`
            }
          }
          console.log(
            `Max claim fee exceeded. Max: ${maxFeeStr}, Required: ${deposit.claimError.requiredFeeSats} sats or ${deposit.claimError.requiredFeeRateSatPerVbyte} sats/vByte`
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

const handleFeeExceeded = async (client: BreezClient, deposit: DepositInfo) => {
  // ANCHOR: handle-fee-exceeded
  if (deposit.claimError?.type === 'maxDepositClaimFeeExceeded') {
    const requiredFee = deposit.claimError.requiredFeeSats

    // Show UI to user with the required fee and get approval
    const userApproved = true // Replace with actual user approval logic

    if (userApproved) {
      const claimRequest: ClaimDepositRequest = {
        txid: deposit.txid,
        vout: deposit.vout,
        maxFee: { type: 'fixed', amount: requiredFee }
      }
      await client.deposits.claim(claimRequest)
    }
  }
  // ANCHOR_END: handle-fee-exceeded
}

const refundDeposit = async (client: BreezClient) => {
  // ANCHOR: refund-deposit
  const txid = 'your_deposit_txid'
  const vout = 0
  const destinationAddress = 'bc1qexample...' // Your Bitcoin address

  // Set the fee for the refund transaction using the half-hour feerate
  const recommendedFees = await client.deposits().recommendedFees()
  const fee: Fee = { type: 'rate', satPerVbyte: recommendedFees.halfHourFee }
  // or using a fixed amount
  // const fee: Fee = { type: 'fixed', amount: 500 }
  //

  const request: RefundDepositRequest = {
    txid,
    vout,
    destinationAddress,
    fee
  }

  const response = await client.deposits.refund(request)
  console.log('Refund transaction created:')
  console.log('Transaction ID:', response.txId)
  console.log('Transaction hex:', response.txHex)
  // ANCHOR_END: refund-deposit
}

const setMaxFeeToRecommendedFees = () => {
  // ANCHOR: set-max-fee-to-recommended-fees
  // Create the default config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Set the maximum fee to the fastest network recommended fee at the time of claim
  // with a leeway of 1 sats/vbyte
  config.maxDepositClaimFee = { type: 'networkRecommended', leewaySatPerVbyte: 1 }
  // ANCHOR_END: set-max-fee-to-recommended-fees
  console.log('Config:', config)
}

const customClaimLogic = async (client: BreezClient, deposit: DepositInfo) => {
  // ANCHOR: custom-claim-logic
  if (deposit.claimError?.type === 'maxDepositClaimFeeExceeded') {
    const requiredFeeRate = deposit.claimError.requiredFeeRateSatPerVbyte

    const recommendedFees = await client.deposits().recommendedFees()

    if (requiredFeeRate <= recommendedFees.fastestFee) {
      const claimRequest: ClaimDepositRequest = {
        txid: deposit.txid,
        vout: deposit.vout,
        maxFee: { type: 'rate', satPerVbyte: requiredFeeRate }
      }
      await client.deposits.claim(claimRequest)
    }
  }
  // ANCHOR_END: custom-claim-logic
}

const exampleRecommendedFees = async (client: BreezClient) => {
  // ANCHOR: recommended-fees
  const response = await client.deposits().recommendedFees()
  console.log('Fastest fee:', response.fastestFee, 'sats/vByte')
  console.log('Half-hour fee:', response.halfHourFee, 'sats/vByte')
  console.log('Hour fee:', response.hourFee, 'sats/vByte')
  console.log('Economy fee:', response.economyFee, 'sats/vByte')
  console.log('Minimum fee:', response.minimumFee, 'sats/vByte')
  // ANCHOR_END: recommended-fees
}
