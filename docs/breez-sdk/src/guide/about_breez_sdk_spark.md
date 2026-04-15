# About Breez SDK - Spark

## **Overview**

The Breez SDK provides developers with an end-to-end solution for integrating instant, non-custodial bitcoin into their apps and services. It eliminates the need for third parties, simplifies the complexities of Bitcoin and Lightning, and enables seamless onboarding for billions of users to the future of value transfer.

## **What is the Breez SDK?**

It’s a nodeless integration that offers a non-custodial, end-to-end solution for integrating bitcoin, utilizing the Bitcoin-native Layer 2 Lightning & Spark, with on-chain interoperability. Using the Breez SDK, you’ll be able to:
- **Send payments** via various protocols such as: Lightning address, LNURL-Pay, Bolt11, BTC address, Spark address, BTKN
- **Receive payments** via various protocols such as: Lightning address, LNURL-Pay, Bolt11, BTC address, Spark address, BTKN
  
**Key Features**

- [x] Send and receive Lightning payments
- [x] Send and receive via LNURL-pay & Lightning addresses 
- [x] Send and receive Spark payments (BTC)
- [x] Passkey login for seedless experience
- [x] Stable Balance - hold your balance in USD
- [x] Issue, send and receive Spark tokens (BTKN)
- [x] On-chain interoperability
- [x] Convert Spark tokens (BTKN) to bitcoin and vice versa
- [x] Bindings to all popular languages & frameworks
- [x] Keys are only held by users
- [x] Multi-app & multi-device support via real-time sync service 
- [x] Payments persistency including restore support
- [x] Automatic claims
- [x] WebAssembly support
- [x] Compatible with external signers
- [x] Free open-source solution

## Pricing

The Breez SDK is **free** for developers. 

## Support

Have a question for the team? Join us on [Telegram](https://t.me/breezsdk) or email us at <contact@breez.technology>.

## API Key

The Breez SDK API key must be set for the SDK to work.

You can request one by <a target="_blank" href="{{api_key_form_uri}}">filling out this form</a> or programmatically with the following request:  

```bash
curl -d "fullname=<full name>" -d "company=<company>" -d "email=<email>" -d "message=<message>" \
  https://breez.technology/contact/apikey
```

The API key is sent to the provided email address.


## Repository

Head over to the <a href="https://github.com/breez/spark-sdk" target="_blank">Breez SDK - Spark</a> repo.


## Next Steps
Follow our step-by-step guide to add the Breez SDK to your app.

**→ [Getting Started](/guide/getting_started.md)** 
