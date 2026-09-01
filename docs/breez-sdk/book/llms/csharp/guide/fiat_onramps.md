# Fiat on-ramps

The SDK integrates hosted fiat rails so a user can fund a flow with a card or bank payment instead of an existing wallet balance. Each returns a URL the user opens to complete the payment.

- [Buying Bitcoin](./buy_bitcoin.md) tops up the user's own Spark wallet with BTC.
- [Prepare Payment Link](./prepare_payment_link.md) sends USDC or USDT to a recipient on an external chain (Ethereum-family, Solana, or Tron).

The [UX guidance](./buy_bitcoin.md#recommended-ux) for opening these URLs (mobile redirect, desktop QR code, pre-opening a browser tab on web) applies to both.
