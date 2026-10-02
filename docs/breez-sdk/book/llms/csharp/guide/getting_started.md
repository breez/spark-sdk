# Getting Started

Integrating Breez SDK into your application takes just a few minutes. Follow these steps to get started:

- **[Installing the SDK](/llms/csharp/guide/install.md)**
- **[Testing and development](/llms/csharp/guide/testing.md)**
- **[Initializing the SDK](/llms/csharp/guide/initializing.md)**
  - **[Customizing the SDK](/llms/csharp/guide/customizing.md)**
- **[Getting the SDK info](/llms/csharp/guide/get_info.md)**
- **[Listening to events](/llms/csharp/guide/events.md)**
- **[Adding logging](/llms/csharp/guide/logging.md)**
- **[Spark status](/llms/csharp/guide/spark_status.md)**

## Languages & Frameworks

The Breez SDK is available for the following languages and frameworks:

<!-- cards: tiles -->
- [iOS/Swift](/llms/csharp/guide/install_ios_swift.md)
- [Android/Kotlin](/llms/csharp/guide/install_android_kotlin.md)
- [Kotlin Multiplatform](/llms/csharp/guide/install_kotlin_multiplatform.md)
- [Javascript/Typescript (Wasm)](/llms/csharp/guide/install_javascript.md)
- [React Native/Expo](/llms/csharp/guide/install_react_native.md)
- [Rust](/llms/csharp/guide/install_rust.md)
- [Flutter](/llms/csharp/guide/install_flutter.md)
- [Go](/llms/csharp/guide/install_go.md)
- [Python](/llms/csharp/guide/install_python.md)
- [C#](/llms/csharp/guide/install_csharp.md)

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

When implementing the Breez SDK, we recommend reading through our [UX Guidelines](/llms/csharp/guide/uxguide.md) to provide a consistent and intuitive experience for your end-users.

Many of the guidelines are implemented in [Glow](https://glow-app.co), which you can use as a UX reference during SDK implementation.

## Demo

Looking for a quick way to try the SDK in your browser or as PWA? Check out our demo app *Glow*:

- **Live demo:** [https://glow-app.co](https://glow-app.co)
- **Repo:** [breez/breez-sdk-spark-example](https://github.com/breez/breez-sdk-spark-example)  

> **Note:** The demo is for demonstration purposes only and not intended for production use.

## Support

Have a question for the team? Join us on [Telegram](https://t.me/breezsdk) or email us at [contact@breez.technology](mailto:contact@breez.technology).
