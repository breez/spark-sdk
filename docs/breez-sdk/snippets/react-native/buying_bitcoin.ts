import {
    type BreezSdk,
    type BuyBitcoinRequest
} from "@breeztech/breez-sdk-spark-react-native"

const buyBitcoinBasic = async (sdk: BreezSdk) => {
    // ANCHOR: buy-bitcoin-basic
    // Buy Bitcoin using the SDK's auto-generated deposit address
    const request: BuyBitcoinRequest = {}

    const response = await sdk.buyBitcoin(request)
    console.log("Open this URL in a browser to complete the purchase:")
    console.log(response.url)
    // ANCHOR_END: buy-bitcoin-basic
}

const buyBitcoinWithAmount = async (sdk: BreezSdk) => {
    // ANCHOR: buy-bitcoin-with-amount
    // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
    const request: BuyBitcoinRequest = {
        lockedAmountSat: 100_000,
    }

    const response = await sdk.buyBitcoin(request)
    console.log("Open this URL in a browser to complete the purchase:")
    console.log(response.url)
    // ANCHOR_END: buy-bitcoin-with-amount
}

const buyBitcoinWithRedirect = async (sdk: BreezSdk) => {
    // ANCHOR: buy-bitcoin-with-redirect
    // Provide a custom redirect URL for after the purchase
    const request: BuyBitcoinRequest = {
        lockedAmountSat: 100_000,
        redirectUrl: "https://example.com/purchase-complete",
    }

    const response = await sdk.buyBitcoin(request)
    console.log("Open this URL in a browser to complete the purchase:")
    console.log(response.url)
    // ANCHOR_END: buy-bitcoin-with-redirect
}
