import {
  type BreezSdk,
  type ListUnclaimedDepositsRequest,
  type ClaimDepositRequest,
  type RefundDepositRequest,
  DepositClaimError,
  Fee,
  Network,
  type DepositInfo,
  recommendedFees,
  Fee_Tags,
  defaultConfig
} from '@breeztech/breez-sdk-spark-react-native'

const listUnclaimedDeposits = async (sdk: BreezSdk) => {
  // ANCHOR: list-unclaimed-deposits
  const request: ListUnclaimedDepositsRequest = {}
  const response = await sdk.listUnclaimedDeposits(request)

  for (const deposit of response.deposits) {
    console.log(`Unclaimed deposit: ${deposit.txid}:${deposit.vout}`)
    console.log(`Amount: ${deposit.amountSats} sats`)

    if (deposit.claimError != null) {
      if (deposit.claimError instanceof DepositClaimError.MaxDepositClaimFeeExceeded) {
        let maxFeeStr = 'none'
        if (deposit.claimError.inner.maxFee != null) {
          if (deposit.claimError.inner.maxFee.tag === Fee_Tags.Fixed) {
            maxFeeStr = `${deposit.claimError.inner.maxFee.inner.amount} sats`
          } else if (deposit.claimError.inner.maxFee.tag === Fee_Tags.Rate) {
            maxFeeStr = `${deposit.claimError.inner.maxFee.inner.satPerVbyte} sats/vByte`
          }
        }
        console.log(
          `Max claim fee exceeded. Max: ${maxFeeStr}, 
          Required: ${deposit.claimError.inner.requiredFeeSats} sats 
          or ${deposit.claimError.inner.requiredFeeRateSatPerVbyte} sats/vByte`
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

const handleFeeExceeded = async (sdk: BreezSdk, deposit: DepositInfo) => {
  // ANCHOR: handle-fee-exceeded
  if (deposit.claimError instanceof DepositClaimError.MaxDepositClaimFeeExceeded) {
    const requiredFee = deposit.claimError.inner.requiredFeeSats

    // Show UI to user with the required fee and get approval
    const userApproved = true // Replace with actual user approval logic

    if (userApproved) {
      const claimRequest: ClaimDepositRequest = {
        txid: deposit.txid,
        vout: deposit.vout,
        maxFee: new Fee.Fixed({ amount: requiredFee })
      }
      await sdk.claimDeposit(claimRequest)
    }
  }
  // ANCHOR_END: handle-fee-exceeded
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

  // Set the fee for the refund transaction using a rate
  const fee = new Fee.Rate({ satPerVbyte: BigInt(5) })
  // or using a fixed amount
  // const fee = new Fee.Fixed({ amount: BigInt(500) })

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

const recommendedFeesExample = async () => {
  // ANCHOR: recommended-fees
  const response = await recommendedFees(Network.Mainnet)
  console.log('Fastest fee:', response.fastestFee, 'sats/vByte')
  console.log('Half-hour fee:', response.halfHourFee, 'sats/vByte')
  console.log('Hour fee:', response.hourFee, 'sats/vByte')
  console.log('Economy fee:', response.economyFee, 'sats/vByte')
  console.log('Minimum fee:', response.minimumFee, 'sats/vByte')
  // ANCHOR_END: recommended-fees
}

const setMaxFeeToRecommendedFees = async () => {
  // ANCHOR: set-max-fee-to-recommended-fees
  // Get the current recommended fees
  const fees = await recommendedFees(Network.Mainnet)

  // Create the default config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Set the maximum deposit claim fee to the fastest recommended fee
  config.maxDepositClaimFee = new Fee.Rate({ satPerVbyte: fees.fastestFee })
  // ANCHOR_END: set-max-fee-to-recommended-fees
}
