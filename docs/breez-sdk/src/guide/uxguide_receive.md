## Receiving payments

### UX principles

- Receiving should feel like **sharing an identifier** (akin to an email), not a multi-step process.
- **Lightning first**: other rails are fallbacks, not choices users must make.

### Guidelines

1. **Lightning should be the primary means of receiving payments.** Lightning is the common language of Bitcoin. **On-chain Bitcoin is secondary** and used as an additional way to on-ramp **only if needed**.
2. **Don’t expose underlying implementation addresses** (i.e. Spark) to end users unless absolutely necessary. More options → more confusion. Moreover, until Spark supports dynamic addresses, exposing a Spark address involves privacy trade-offs.
3. **Display an LNURL-Pay QR code** by default (widest supported reusable method).  
   Provide **fallback to BOLT11** for one-off payment requests, typically with a specified amount.
4. **Provide a human-readable Lightning address.** Start with a **random** address that the user can **customize later**.
5. **Expose two primary actions**:
   - **Copy** → copies the Lightning address
   - **Share** → shares the LNURL-Pay string  
     (Matches patterns of popular Lightning wallets and maximizes compatibility.)
6. **If BOLT12 can be supported (currently only on the Liquid implementation)**, use the **same Lightning address** and **enhance it with BIP-353** so the address can retrieve both **LNURL-Pay** (BOLT11 under the hood) **and a BOLT12 offer**.
7. **If there are limits or fees, display them.** Make constraints visible before users attempt payment.
8. Interact with the SDK events according to these [UX recommendations](/guide/receive_payment.md#lightning-1).
