## Sending Payments
### UX Principles
- Provide a **unified entry point** that “just works” regardless of the pasted/scanned data.
- Users shouldn’t need to recognize standards (BOLT11 vs. LNURL vs. address).

### Guidelines
1. **Consolidate all sending methods in the same UI** (BOLT11, Lightning address, BTC address, LNURL, etc.).
2. Add **On-chain Bitcoin** to the above UI as an additional way to off-ramp **only if needed**.
3. Support **Paste**, **Scan** (external QR), and **Upload** (scan from photos/screenshots).
4. **Contacts:** Allow users to **save Lightning addresses** in a manageable **Contacts** list to improve the send experience (not yet implemented in Misty Breez, but recommended).
5. Provide **Use all funds** when paying to a Lightning or Bitcoin address.
6. **Validate and display amount limits and fees** before confirmation.
7. Interact with the SDK events according to these [UX recommendations](/guide/send_payment.md#lightning-2). 
