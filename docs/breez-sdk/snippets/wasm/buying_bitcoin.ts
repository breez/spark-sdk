import {
  type BreezClient,
  type BuyBitcoinRequest
} from '@breeztech/breez-sdk-spark'

const buyBitcoin = async (client: BreezClient) => {
  // ANCHOR: buy-bitcoin
  // Optionally, lock the purchase to a specific amount
  const optionalLockedAmountSat = 100_000
  // Optionally, set a redirect URL for after the purchase is completed
  const optionalRedirectUrl = 'https://example.com/purchase-complete'

  const request: BuyBitcoinRequest = {
    lockedAmountSat: optionalLockedAmountSat,
    redirectUrl: optionalRedirectUrl
  }

  const response = await client.buyBitcoin(request)
  console.log('Open this URL in a browser to complete the purchase:')
  console.log(response.url)
  // ANCHOR_END: buy-bitcoin
}
