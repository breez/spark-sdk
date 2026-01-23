import { BreezSdk, BuyBitcoinRequest } from "@breez/react-native-breez-sdk-spark"

const buyBitcoinBasic = async (sdk: BreezSdk) => {
    // ANCHOR: buy-bitcoin-basic
    const request: BuyBitcoinRequest = {
        address: "bc1qexample...", // Your Bitcoin address
        lockedAmountSat: undefined,
        maxAmountSat: undefined,
        redirectUrl: undefined,
    }

    const response = await sdk.buyBitcoin(request)
    console.log("Open this URL in a browser to complete the purchase:")
    console.log(response.url)
    // ANCHOR_END: buy-bitcoin-basic
}

const buyBitcoinWithAmount = async (sdk: BreezSdk) => {
    // ANCHOR: buy-bitcoin-with-amount
    // Lock the purchase to a specific amount (e.g., 0.001 BTC = 100,000 sats)
    const request: BuyBitcoinRequest = {
        address: "bc1qexample...",
        lockedAmountSat: 100_000, // Pre-fill with 100,000 sats
        maxAmountSat: undefined,
        redirectUrl: undefined,
    }

    const response = await sdk.buyBitcoin(request)
    console.log("Open this URL in a browser to complete the purchase:")
    console.log(response.url)
    // ANCHOR_END: buy-bitcoin-with-amount
}

const buyBitcoinWithLimits = async (sdk: BreezSdk) => {
    // ANCHOR: buy-bitcoin-with-limits
    // Set both a locked amount and maximum amount
    const request: BuyBitcoinRequest = {
        address: "bc1qexample...",
        lockedAmountSat: 50_000,   // Pre-fill with 50,000 sats
        maxAmountSat: 500_000,     // Limit to 500,000 sats max
        redirectUrl: undefined,
    }

    const response = await sdk.buyBitcoin(request)
    console.log("Open this URL in a browser to complete the purchase:")
    console.log(response.url)
    // ANCHOR_END: buy-bitcoin-with-limits
}

const buyBitcoinWithRedirect = async (sdk: BreezSdk) => {
    // ANCHOR: buy-bitcoin-with-redirect
    // Provide a custom redirect URL for after the purchase
    const request: BuyBitcoinRequest = {
        address: "bc1qexample...",
        lockedAmountSat: 100_000,
        maxAmountSat: undefined,
        redirectUrl: "https://example.com/purchase-complete",
    }

    const response = await sdk.buyBitcoin(request)
    console.log("Open this URL in a browser to complete the purchase:")
    console.log(response.url)
    // ANCHOR_END: buy-bitcoin-with-redirect
}
