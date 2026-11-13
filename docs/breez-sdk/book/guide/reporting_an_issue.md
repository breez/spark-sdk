# Reporting an issue

Most issues are resolved by the three checks below. Go through them first, then report what is left.

## Before you report

### 1. Check the documentation

This guide covers setup, API keys, every feature, and the [errors](errors.md) the SDK returns. Requesting an API key is self-service: see [API Key](about_breez_sdk_spark.md#api-key).

If you are working with an AI coding agent, point it at these three sources. Together they answer most integration questions:

- **This documentation:** [docs.breez.technology](https://docs.breez.technology/)
- **The SDK repository:** [breez/spark-sdk](https://github.com/breez/spark-sdk)
- **The Glow web repository:** [breez/glow-web](https://github.com/breez/glow-web)

### 2. Test on the latest version

Always reproduce the issue on the [latest release](https://github.com/breez/spark-sdk/releases) of the SDK before reporting it. Many issues are already fixed there.

### 3. Compare with Glow

[Glow](https://breez.technology/glow) is our open-source reference app, built on the SDK and available on web, iOS, and Android. If something isn't working in your app, try the same flow in Glow:

- **It works in Glow:** the difference is in your integration. Compare your code with the Glow source: [breez/glow-web](https://github.com/breez/glow-web) for web, [breez/glow-app](https://github.com/breez/glow-app) for iOS and Android.
- **It fails in Glow too:** report it, and mention that Glow shows the same behavior.

## What to include

- The SDK version, language, and platform
- The steps to reproduce the issue, with what you expected and what happened instead
- The error returned by the SDK
- SDK logs up to and including **DEBUG** level, covering the time of the issue. See [Adding logging](logging.md).
- Whether the same flow works in Glow

## Where to report

- **Telegram:** join our public group at [t.me/breezsdk](https://t.me/breezsdk)
- **Email:** [contact@breez.technology](mailto:contact@breez.technology)
- **GitHub:** open an issue at [breez/spark-sdk](https://github.com/breez/spark-sdk/issues)