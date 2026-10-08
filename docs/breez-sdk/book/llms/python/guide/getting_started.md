# Getting Started

Integrating Breez SDK into your product takes just a few minutes. Follow these steps to get started:

<!-- cards: links -->
- **[Installing the SDK](/llms/python/guide/install.md)**
- **[Testing and development](/llms/python/guide/testing.md)**
- **[Initializing the SDK](/llms/python/guide/initializing.md)**
  - **[Customizing the SDK](/llms/python/guide/customizing.md)**
- **[Getting the SDK info](/llms/python/guide/get_info.md)**
- **[Listening to events](/llms/python/guide/events.md)**
- **[Adding logging](/llms/python/guide/logging.md)**
- **[Spark status](/llms/python/guide/spark_status.md)**

## Languages & Frameworks

The Breez SDK is available for the following languages and frameworks:

<!-- cards: tiles -->
- [iOS/Swift](/llms/python/guide/install_ios_swift.md)
- [Android/Kotlin](/llms/python/guide/install_android_kotlin.md)
- [Kotlin Multiplatform](/llms/python/guide/install_kotlin_multiplatform.md)
- [Javascript/Typescript (Wasm)](/llms/python/guide/install_javascript.md)
- [React Native/Expo](/llms/python/guide/install_react_native.md)
- [Rust](/llms/python/guide/install_rust.md)
- [Flutter](/llms/python/guide/install_flutter.md)
- [Go](/llms/python/guide/install_go.md)
- [Python](/llms/python/guide/install_python.md)
- [C#](/llms/python/guide/install_csharp.md)

## API Key

The Breez SDK API key must be set for the SDK to work. The API key is sent to the provided email address.

<a class="doc-button" target="_blank" href="https://breez.technology/request-api-key/#contact-us-form-sdk">Request an API key</a>

Or request one programmatically with the following request:

```bash
curl https://breez.technology/contact/apikey \
  -d "fullname=<full name>" \
  -d "company=<company>" \
  -d "email=<email>" \
  -d "message=<message>"
```

## UX Guidelines

When implementing the Breez SDK, we recommend reading through our [UX Guidelines](/llms/python/guide/uxguide.md) to provide a consistent and intuitive experience for your end-users.

Many of the guidelines are implemented in [Glow](https://breez.technology/glow), which you can use as a UX reference during SDK implementation.

## Demo

Glow is a bitcoin and stablecoins app, powered by the Breez SDK. Users experience the best of bitcoin UX. Developers get a production-ready reference to explore, fork, and ship.

<!-- cards: stores -->
- [Download Glow on the App Store](https://apps.apple.com/us/app/glow-lightning-fast-bitcoin/id6762465698)
- [Get Glow on Google Play](https://play.google.com/store/apps/details?id=technology.breez.glow)
- [Repository](https://github.com/breez/glow-app)

Looking for a quick way to try the SDK in your browser or as PWA? Check out the Glow web demo:

- **Live demo:** [https://glow-app.co](https://glow-app.co)
- **Repository:** [breez/glow-web](https://github.com/breez/glow-web)

> **Note:** The web demo is for demonstration purposes only and not intended for production use.

## Support

Have a question for the team? Join us on [Telegram](https://t.me/breezsdk) or email us at [contact@breez.technology](mailto:contact@breez.technology).
