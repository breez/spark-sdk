# Getting Started

Integrating Breez SDK into your application takes just a few minutes. Follow these steps to get started:

- **[Installing the SDK](/llms/react-native/guide/install.md)**
- **[Testing and development](/llms/react-native/guide/testing.md)**
- **[Initializing the SDK](/llms/react-native/guide/initializing.md)**
  - **[Customizing the SDK](/llms/react-native/guide/customizing.md)**
- **[Getting the SDK info](/llms/react-native/guide/get_info.md)**
- **[Listening to events](/llms/react-native/guide/events.md)**
- **[Adding logging](/llms/react-native/guide/logging.md)**
- **[Spark status](/llms/react-native/guide/spark_status.md)**

## API Key

The Breez SDK API key must be set for the SDK to work. You can request one by <a target="_blank" href="https://breez.technology/request-api-key/#contact-us-form-sdk">filling out this form</a>, or programmatically with the following request:

```bash
curl -d "fullname=<full name>" -d "company=<company>" -d "email=<email>" -d "message=<message>" \
  https://breez.technology/contact/apikey
```

The API key is sent to the provided email address.

## UX Guidelines

When implementing the Breez SDK, we recommend reading through our [UX Guidelines](/llms/react-native/guide/uxguide.md) to provide a consistent and intuitive experience for your end-users.

Many of the guidelines are implemented in [Glow](https://glow-app.co), which you can use as a UX reference during SDK implementation.

## Demo

Looking for a quick way to try the SDK in your browser or as PWA? Check out our demo app *Glow*:

- **Live demo:** [https://glow-app.co](https://glow-app.co)
- **Repo:** [breez/breez-sdk-spark-example](https://github.com/breez/breez-sdk-spark-example)  

> **Note:** The demo is for demonstration purposes only and not intended for production use.

## Support

Have a question for the team? Join us on [Telegram](https://t.me/breezsdk) or email us at [contact@breez.technology](mailto:contact@breez.technology).
