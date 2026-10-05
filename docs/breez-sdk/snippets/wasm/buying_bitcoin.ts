import {
  type BreezSdk
} from '@breeztech/breez-sdk-spark'

const buyBitcoin = async (sdk: BreezSdk) => {
  // ANCHOR: buy-bitcoin
  // Optionally, prefill the purchase amount
  const optionalAmountSat = 100_000
  // Optionally, set a redirect URL for after the purchase is completed
  const optionalRedirectUrl = 'https://example.com/purchase-complete'

  const response = await sdk.buyBitcoin({
    type: 'moonpay',
    delivery: {
      type: 'bitcoin',
      amountSat: optionalAmountSat
    },
    redirectUrl: optionalRedirectUrl
  })
  console.log('Open this URL in a browser to complete the purchase:')
  console.log(response.url)
  // ANCHOR_END: buy-bitcoin
}

const buyBitcoinViaCrossChain = async (sdk: BreezSdk) => {
  // ANCHOR: buy-bitcoin-cross-chain
  // USD amount to receive, in 6-decimal base units ($50)
  const amount = '50000000'

  const response = await sdk.buyBitcoin({
    type: 'moonpay',
    delivery: {
      type: 'crossChain',
      amount,
      feeMode: undefined
    },
    redirectUrl: undefined
  })
  console.log('Open this URL in a browser to complete the purchase:')
  console.log(response.url)

  if (response.crossChainInfo !== undefined) {
    const info = response.crossChainInfo
    console.log(`USDC to buy: ${info.depositAmount}`)
    console.log(
      `Expected to receive: ${info.expectedReceivedAmount} ${info.destinationAsset}`
    )
    console.log(`Conversion fee: ${info.serviceFeeAmount}`)
  }
  // ANCHOR_END: buy-bitcoin-cross-chain
}

const buyBitcoinViaCashapp = async (sdk: BreezSdk) => {
  // ANCHOR: buy-bitcoin-cashapp
  // Cash App requires the amount to be specified up front.
  const amountSats = 50_000

  const response = await sdk.buyBitcoin({
    type: 'cashApp',
    amountSats
  })
  console.log('Open this URL in Cash App to complete the purchase:')
  console.log(response.url)
  // ANCHOR_END: buy-bitcoin-cashapp
}
