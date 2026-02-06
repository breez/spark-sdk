# Buying Bitcoin

The Breez SDK provides a simple way to enable users to purchase Bitcoin through external providers like MoonPay. Users are directed to a provider URL in their browser, where they can complete the purchase and have funds sent directly to an automatically generated deposit address.

## Basic usage

To initiate a Bitcoin purchase, call the `buy_bitcoin` method:

{{#tabs buying_bitcoin:buy-bitcoin-basic}}

The method returns a URL that the user should open in a browser to complete the purchase with the provider.

## Locking an amount

You can pre-fill the purchase amount by specifying a `locked_amount_sat` parameter. This locks the user to a specific amount during the purchase process:

{{#tabs buying_bitcoin:buy-bitcoin-with-amount}}

## Custom redirect URL

Provide a custom redirect URL to direct the user to a specific page after completing the purchase:

{{#tabs buying_bitcoin:buy-bitcoin-with-redirect}}

## Integration with your app

Here's a typical integration pattern:

1. **Call `buy_bitcoin`** - Initiate the purchase request with optional parameters
2. **Open the URL** - Display the returned URL to the user in a browser
3. **Handle the result** - The user completes the purchase on the provider's site and is redirected

The funds purchased by the user will be sent directly to an automatically generated deposit address on the blockchain.
