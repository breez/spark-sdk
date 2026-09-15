## Displaying payments

### UX principles

- History should be **clear, transparent, and verifiable**.
- Offer both **simple summaries** and **deeper technical details**: a glanceable list, with proof underneath for whoever needs it.

### Guidelines

1. **Display fees separately** from the amount, never silently folded in.
2. **Name payments after people.** Prefer the contact name, then the Lightning address, over the invoice description in payment titles. Resolve contact names when rendering, so renaming a contact updates their whole history; keep the raw address available in the details view.
3. **Keep the proof one tap away.** Show the payment's metadata, at minimum the invoice and preimage, under an expandable details section so the list stays uncluttered but every payment remains verifiable.
4. **Make state unmistakable**: pending, succeeded, and failed payments each get distinct visuals. A payment whose conversion or cross-chain delivery is still in flight is pending, even though the user's part is done.
5. **Show conversions honestly.** When the user converts between bitcoin and USD, that's an event they initiated, so it appears in the history (e.g. "Conversion to USD"). When a conversion is merely the internal step of a payment in progress, it doesn't: the user sees one payment, not its plumbing. Denominate stablecoin-related payments in USD, and put the conversion breakdown (provider, amounts, fee) under the details section.
6. **Lead with what the recipient got.** For USDC/USDT sends, the details view opens with the delivered amount, network, recipient address, and provider, ahead of the internal transfer metadata.
