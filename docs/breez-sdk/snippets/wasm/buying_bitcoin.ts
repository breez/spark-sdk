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
  const response = await sdk.buyBitcoin({
    type: 'cashApp',
    amountSats: undefined
  })
  console.log('Open this URL in Cash App to complete the purchase:')
  console.log(response.url)
  // ANCHOR_END: buy-bitcoin-cashapp
}
