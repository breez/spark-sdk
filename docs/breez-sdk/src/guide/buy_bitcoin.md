# Buying Bitcoin

The Breez SDK provides a simple way to enable users to purchase Bitcoin through external providers like MoonPay. Users are directed to a provider URL in their browser, where they can complete the purchase and have the funds deposited directly into their wallet.

## Usage

To initiate a Bitcoin purchase, call the `buy_bitcoin` method with an optional `locked_amount_sat` to pre-fill the amount and an optional `redirect_url` to direct the user after the purchase:

{{#tabs buying_bitcoin:buy-bitcoin}}

The method returns a URL that the user should open in a browser to complete the purchase with the provider.

## Integration with your app

Here's a typical integration pattern:

1. **Call `buy_bitcoin`** - Initiate the purchase request with optional parameters
2. **Open the URL** - Display the returned URL to the user in a browser
3. **Handle the result** - The user completes the purchase on the provider's site and is redirected

The purchased funds will be deposited directly into the user's wallet.
