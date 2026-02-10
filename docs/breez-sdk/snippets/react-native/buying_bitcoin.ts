import {
  type BreezSdk,
  type BuyBitcoinRequest
} from '@breeztech/breez-sdk-spark-react-native'

const buyBitcoin = async (sdk: BreezSdk) => {
  // ANCHOR: buy-bitcoin
  // Buy Bitcoin with funds deposited directly into the user's wallet.
  // Optionally lock the purchase to a specific amount and provide a redirect URL.
  const request: BuyBitcoinRequest = {
    lockedAmountSat: BigInt(100_000),
    redirectUrl: 'https://example.com/purchase-complete'
  }

  const response = await sdk.buyBitcoin(request)
  console.log('Open this URL in a browser to complete the purchase:')
  console.log(response.url)
  // ANCHOR_END: buy-bitcoin
}
