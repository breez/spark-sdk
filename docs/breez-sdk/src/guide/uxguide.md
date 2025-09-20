# UX Guidelines
These guidelines describe how to integrate the Breez SDK to create a consistent UI/UX that feels natural for end users. They are based on [Misty Breez](https://breez.technology/misty/) patterns and are recommendations you can adapt to your specific use cases.
> **Reference:** Many of these guidelines are implemented in **Misty Breez**. Use it as the primary UX reference during SDK implementation.

## Core UX Principles
- **Simplicity over choice** — Users should not have to pick protocols or rails unless absolutely necessary.
- **Transparency without jargon** — Show limits, fees, and conditions up front in plain language.
- **Progressive disclosure** — Keep advanced details available but tucked away by default.

## Receiving Payments
### UX Principles
- Receiving should feel like **sharing an identifier** (akin to an email), not a multi-step process.
- **Lightning first**: other rails are fallbacks, not choices users must make.
  
### Guidelines
1. **Lightning should be the primary means of receiving payments.** Lightning is the common language of Bitcoin. **On-chain Bitcoin is secondary** and used as an additional way to on-ramp **only if needed**.
2. **Don’t expose underlying implementation addresses** (i.e. Spark) to end users unless absolutely necessary. More options → more confusion.
3. **Display an LNURL-Pay QR code** by default (widest supported reusable method).  
   Provide **fallback to BOLT11** for one-off payment requests, typically with a specified amount.
4. **Provide a human-readable Lightning address.** Start with a **random** address that the user can **customize later**.
5. **Expose two primary actions**:  
   - **Copy** → copies the Lightning address  
   - **Share** → shares the LNURL-Pay string  
   (Matches patterns of popular Lightning wallets and maximizes compatibility.)
6. **If BOLT12 can be supported (currently only on the Liquid implementation)**, use the **same Lightning address** and **enhance it with BIP-353** so the address can retrieve both **LNURL-Pay** (BOLT11 under the hood) **and a BOLT12 offer**.
7. **If there are limits or fees, display them.** Make constraints visible before users attempt payment.

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
   
## Displaying Payments
### UX Principles
- History should be **clear, transparent, and verifiable**.
- Offer both **simple summaries** and **deeper technical details**.

### Guidelines
1. **Display fees separately** from the amount.
2. **Prefer the Lightning address** over the invoice description in titles for readability.
3. **Show associated metadata** — at minimum the **invoice** and **preimage**.  
   Keep under an expandable **Details** section to avoid clutter by default.
4. **Represent states:** **Pending**, **Succeeded**, **Failed** (with distinct visuals).

## Seed & Key Management

### UX Principles
- Onboarding should be **seamless**; don’t block first use with seed backup.
- Encourage security without forcing it prematurely.

### Guidelines
1. **Allow seed backup after wallet creation** or **after the first received payment** to keep onboarding smooth.
2. **Explain** the need to **write down and save** the seed phrase.
3. **Consider validation** (e.g., partial re-entry) to confirm backup.
4. **Enhance key-management UX where possible:** encrypted cloud backup, or **Web 2 / identity–based** approaches that preserve self-custody.
