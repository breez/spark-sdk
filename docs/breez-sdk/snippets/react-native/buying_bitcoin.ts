import {
  type BreezSdk,
  BuyBitcoinRequest,
  MoonpayDelivery
} from '@breeztech/breez-sdk-spark-react-native'

const buyBitcoin = async (sdk: BreezSdk) => {
  // ANCHOR: buy-bitcoin
  // Optionally, prefill the purchase amount
  const optionalAmountSat = BigInt(100_000)
  // Optionally, set a redirect URL for after the purchase is completed
  const optionalRedirectUrl = 'https://example.com/purchase-complete'

  const request = new BuyBitcoinRequest.Moonpay({
    delivery: new MoonpayDelivery.Bitcoin({ amountSat: optionalAmountSat }),
    redirectUrl: optionalRedirectUrl
  })

  const response = await sdk.buyBitcoin(request)
  console.log('Open this URL in a browser to complete the purchase:')
  console.log(response.url)
  // ANCHOR_END: buy-bitcoin
}

const buyBitcoinViaCrossChain = async (sdk: BreezSdk) => {
  // ANCHOR: buy-bitcoin-cross-chain
  // USD amount to receive, in 6-decimal base units ($50)
  const amount = BigInt(50_000_000)

  const request = new BuyBitcoinRequest.Moonpay({
    delivery: new MoonpayDelivery.CrossChain({ amount, feeMode: undefined }),
    redirectUrl: undefined
  })

  const response = await sdk.buyBitcoin(request)
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
  const amountSats = BigInt(50_000)

  const request = new BuyBitcoinRequest.CashApp({
    amountSats
  })

  const response = await sdk.buyBitcoin(request)
  console.log('Open this URL in Cash App to complete the purchase:')
  console.log(response.url)
  // ANCHOR_END: buy-bitcoin-cashapp
}
