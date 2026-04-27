import {
  type BreezSdk,
  BuyBitcoinRequest
} from '@breeztech/breez-sdk-spark-react-native'

const buyBitcoin = async (sdk: BreezSdk) => {
  // ANCHOR: buy-bitcoin
  // Optionally, lock the purchase to a specific amount
  const optionalLockedAmountSat = BigInt(100_000)
  // Optionally, set a redirect URL for after the purchase is completed
  const optionalRedirectUrl = 'https://example.com/purchase-complete'

  const request = new BuyBitcoinRequest.Moonpay({
    lockedAmountSat: optionalLockedAmountSat,
    redirectUrl: optionalRedirectUrl
  })

  const response = await sdk.buyBitcoin(request)
  console.log('Open this URL in a browser to complete the purchase:')
  console.log(response.url)
  // ANCHOR_END: buy-bitcoin
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
