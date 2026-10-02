# About Breez SDK

The Breez SDK is the simplest way to add instant, non‑custodial bitcoin and stablecoins to any app or service, removing third-party dependencies and enabling value to move as easily as information.

Integrate in minutes with just a few lines of code, onboard users anywhere without the licensing burden, and bring value transfer to billions around the world.

## **What is the Breez SDK?**

It’s a nodeless integration that offers a non-custodial, end-to-end solution for integrating bitcoin and stablecoins, utilizing the Bitcoin-native Layer 2 Lightning & Spark, with on-chain interoperability. Using the Breez SDK, you’ll be able to:

- Send and receive bitcoin via Lightning addresses, Bolt11 invoices, LNURL-Pay, bitcoin addresses, and Spark addresses
- Send and receive USDC or USDT on Ethereum, Base, Arbitrum, Solana, Tron, and other networks, to and from a bitcoin or USD balance
- Issue, send, and receive Spark tokens (BTKN)
  
## Key Features

- [x] **[Fully-featured Lightning](/llms/go/guide/lnurl.md)**: Lightning addresses, invoices, LNURL-Pay, -Withdraw, -Auth, and -Verify.
- [x] **[Languages & Frameworks](/llms/go/guide/install.md)**: Kotlin, Swift, JS, React Native, Flutter, Go, Python, C#, and WASM.
- [x] **[Passkey Login](/llms/go/guide/passkey.md)**: Seedless onboarding and restore, no recovery phrase needed.
- [x] **[USDT & USDC](/llms/go/guide/cross_chain.md)**: Users can send/receive stablecoins from their BTC or stable balance.
- [x] **[Stable Balance](/llms/go/guide/stable_balance.md)**: Users can hold their balance in USD to avoid BTC price volatility.
- [x] **[Instant Deposits](/llms/go/guide/onchain_claims.md)**: Claim on-chain deposits in seconds, no confirmations needed.
- [x] **[Multi-device/app Sync](/llms/go/guide/config.md#real-time-sync-server-url)**: Users can access their balance from multiple devices and apps.
- [x] **[Contacts API](/llms/go/guide/contacts.md)**: Save and reuse Lightning addresses, synced across devices.
- [x] **[Caching & Persistence](/llms/go/guide/customizing.md#with-storage)**: Out-of-the-box caching and persistence for a smooth UX.
- [x] **[External Signer](/llms/go/guide/external_signer.md)**: Bring your own key management.
- [x] **[Turnkey Signer](/llms/go/guide/turnkey.md)**: A built-in signer for server-side, non-custodial embedded wallets.
- [x] **[Integrated On-ramps](/llms/go/guide/buy_bitcoin.md)**: Let users buy bitcoin directly via Cash App and MoonPay.
- [x] **[Fiat Currencies](/llms/go/guide/fiat_currencies.md)**: Real-time exchange rates to display value in fiat currencies.
- [x] **[Multi-user Server Mode](/llms/go/guide/server_mode.md)**: Run multiple user balances from a single backend server.

## Pricing

The Breez SDK is **free** for developers. 

## Support

Have a question for the team? Join us on [Telegram](https://t.me/breezsdk) or email us at <contact@breez.technology>.

## API Key

The Breez SDK API key must be set for the SDK to work.

You can request one by <a target="_blank" href="https://breez.technology/request-api-key/#contact-us-form-sdk">filling out this form</a> or programmatically with the following request:  

```bash
curl -d "fullname=<full name>" -d "company=<company>" -d "email=<email>" -d "message=<message>" \
  https://breez.technology/contact/apikey
```

The API key is sent to the provided email address.


## Repository

Head over to the <a href="https://github.com/breez/spark-sdk" target="_blank">Breez SDK</a> repo.


## Next Steps
Follow our step-by-step guide to add the Breez SDK to your app.

**→ [Getting Started](/llms/go/guide/getting_started.md)**
