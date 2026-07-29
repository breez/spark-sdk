## Receiving payments

### UX principles

- Receiving should feel like **sharing an identifier**, akin to sharing an email address, not like executing a multi-step process.
- **Lightning first**: other rails are fallbacks the wallet falls back to, not choices the user must make.

### Guidelines

1. **Make Lightning the primary way to receive.** Lightning is the common language of Bitcoin. Treat on-chain Bitcoin as a secondary on-ramp, offered only if your use case needs it.
2. **Don't expose implementation addresses** (i.e. Spark) unless absolutely necessary. Every extra option adds confusion, and until Spark supports dynamic addresses, exposing a Spark address carries privacy trade-offs.
3. **Show a reusable QR code by default** (LNURL-Pay, the most widely supported reusable method), with a fallback to a BOLT11 invoice for one-off requests with a specific amount.
4. **Give every user a human-readable Lightning address.** Register a random one automatically so receiving works from the first moment, and let the user customize it later. If they change it, warn that the old address is released and may be claimed by someone else.
5. **Expose two primary actions**: **Copy** (the Lightning address) and **Share** (the LNURL-Pay string). This matches the patterns of popular Lightning wallets and maximizes compatibility.
6. **Respect the active denomination.** When [Stable Balance](uxguide_send.md#stable-balance) is active, let users request amounts in USD and announce received payments in USD. The conversion from incoming bitcoin happens automatically; the user just sees dollars arrive.
7. **Show limits and fees before they bite.** If a payment request carries a receive fee or amount limits, display them when the request is created, not after the payment fails.
8. **Reflect payment progress as it happens**, driven by SDK events: see the [receive payment UX recommendations](/guide/receive_payment.md#lightning-1).

### On-chain deposits

The SDK claims incoming on-chain deposits automatically. The UX only needs to surface the exceptions: a deposit still confirming, a claim that costs more than the configured maximum, and a refund the user chose. See [Claiming on-chain deposits](onchain_claims.md).

1. **Deposits are payments.** Show incoming deposits in the payment history as pending entries while they confirm and while claiming runs. Don't invent a separate deposits screen.
2. **Silence is the happy path.** When a deposit confirms and the claim fee is within bounds, the SDK claims it with no user action; the user simply sees the payment complete.
3. **Ask when it costs more than expected.** When claiming would exceed the configured maximum fee, present a clear choice: approve, showing the amount, the network fee, and what the user will actually receive, or reject.
4. **A rejected deposit stays visible until resolved.** Keep a persistent indicator that leads to a refund flow: destination address, a fee-speed choice, and the refund transaction id once broadcast.
