## Sending payments

### UX principles

- Provide a **unified entry point** that "just works" regardless of what the user pastes or scans.
- Users should recognize **people and amounts**, not standards (BOLT11 vs. LNURL vs. address).
- **Denominate in what the user holds**: when Stable Balance is active, amounts are entered in USD.

### Guidelines

1. **One send flow for everything.** A single input accepts BOLT11, Lightning address, Bitcoin address, LNURL, USDC/USDT address, or a contact. Parsing decides what happens next; the user never picks a payment type.
2. Add **on-chain Bitcoin** to that same flow as an off-ramp, only if your use case needs it.
3. **Meet the input where it is**: support Paste, Scan (camera QR), and Upload (QR from photos or screenshots).
4. **Offer "Use all funds"** when paying to a Lightning or Bitcoin address.
5. **Validate early, disclose before commitment.** Check amounts against limits and balance as the user types, and show all fees on the confirmation screen, before anything is sent.
6. **Reflect payment progress as it happens**, driven by SDK events: see the [send payment UX recommendations](/guide/send_payment.md#lightning-2).

### Contacts

Sending should feel like **paying a person**, not pasting a string. Contacts (a name plus a Lightning address, synced across the user's devices by the SDK) are how the wallet gets there. See [Managing contacts](contacts.md).

1. **Contacts are people, so they live where people are paid**: inside the send flow, as a Contacts action next to Paste and Scan, and as autocomplete suggestions while the user types. Once chosen, show the contact (name and address) as a single clearable unit rather than raw text, and confirm as "Pay to {name}".
2. **Grow the contact list from real payments.** After a successful payment to a Lightning address the user hasn't saved, offer to save it: a non-blocking prompt after the send flow closes, with the name pre-filled from the address (`alice` for `alice@domain.com`). Never interrupt the payment itself.
3. **Only save addresses that work.** Before saving, verify the Lightning address actually resolves (using {{#name parse}}), not just that it looks valid.
4. **Trust the sync, but wait for it.** Refresh the contact list on each {{#enum SdkEvent::Synced}} event so edits from other devices appear on their own, and don't show "no contacts yet" until the first sync has completed, so an existing user's contacts never look lost on a new device.

### Stable Balance

[Stable Balance](stable_balance.md) means the wallet holds **a single balance, in either bitcoin or USD**. It is a denomination the user chooses, not a second account, and every screen should reflect that choice.

1. **One balance.** Show the balance in the active denomination only: USD when Stable Balance is active, sats otherwise. Never show two balances side by side. Residual sats below the conversion threshold appear as "change" under the USD balance.
2. **Switching denomination is switching the balance.** Let the user flip between BTC and USD from the balance itself, with a one-time explainer and the conversion fee shown before confirming. If a wallet holds a stablecoin balance while the mode is off (e.g. after restoring on another device), prompt to switch back to USD.
3. **The denomination carries through the flow.** When Stable Balance is active, amounts are entered and displayed in USD by default, with a switcher to sats.
4. **Conversions stay invisible until they cost something.** The user still sends and receives bitcoin; the SDK converts under the hood when preparing the payment. Disclose the conversion fee on the confirmation screen when the prepare response includes a {{#name conversion_estimate}}, and show "Converting..." before "Sending..." during execution.

### Sending USDC/USDT

Frame [cross-chain sends](cross_chain.md) as **sending dollars**, not as moving crypto between chains.

1. **Same flow, different destination.** A pasted or scanned USDC/USDT address (EVM, Solana, or Tron, detected by {{#name parse}}) enters through the same unified send flow as everything else. No dedicated "cross-chain" screen.
2. **Dollars in, dollars out.** Denominate the amount in USD only; sats never appear in this flow.
3. **Ask only what can't be inferred.** When the destination is ambiguous, let the user pick the asset, then the network, then the provider, and skip any step that has a single option. Show the token contract address on request so careful users can verify the destination asset.
4. **Let providers compete in the open.** When multiple providers serve a route, show each one's receive amount and fee side by side and let the user choose. Hide providers that fail to produce a quote.
5. **Confirm with a full breakdown**: the amount the recipient receives, chain, provider, recipient address, and fee in the destination asset. Quotes expire; when one does, fetch a fresh one rather than sending on stale numbers.
6. **Sending is fast, delivery takes time.** The user's part completes quickly; the cross-chain delivery continues in the background. Show the payment as pending in the history until delivery completes, and rely on the SDK's automatic refund when a delivery fails.
