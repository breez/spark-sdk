import {
  type BreezSdk
} from '@breeztech/breez-sdk-spark'

const buyBitcoin = async (sdk: BreezSdk) => {
  // ANCHOR: buy-bitcoin
  // Optionally, lock the purchase to a specific amount
  const optionalLockedAmountSat = 100_000
  // Optionally, set a redirect URL for after the purchase is completed
  const optionalRedirectUrl = 'https://example.com/purchase-complete'

  const response = await sdk.buyBitcoin({
    type: 'moonpay',
    lockedAmountSat: optionalLockedAmountSat,
    redirectUrl: optionalRedirectUrl
  })
  console.log('Open this URL in a browser to complete the purchase:')
  console.log(response.url)
  // ANCHOR_END: buy-bitcoin
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
