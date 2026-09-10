# React Native/Expo Managed Workflow

We recommend using the official npm package: [@breeztech/breez-sdk-spark-react-native](https://www.npmjs.com/package/@breeztech/breez-sdk-spark-react-native).

## React Native

```console
npm install @breeztech/breez-sdk-spark-react-native
```
or
```console
yarn add @breeztech/breez-sdk-spark-react-native
```

## Expo Managed Workflow

```console
npx expo install @breeztech/breez-sdk-spark-react-native
```

Add the plugin to your `app.json` or `app.config.js`:

```json
{
  "expo": {
    "plugins": [
      "@breeztech/breez-sdk-spark-react-native"
    ]
  }
}
```

### Plugin Options

To enable [Passkey](passkey_setup.md#ios--macos-apple-app-site-association) support, set `enablePasskey` to `true`. Your app must have the <a target="_blank" href="https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.associated-domains">Associated Domains</a> capability enabled. This adds `webcredentials:keys.breez.technology` to the iOS Associated Domains entitlement:

```json
{
  "expo": {
    "plugins": [
      ["@breeztech/breez-sdk-spark-react-native", {
        "enablePasskey": true
      }]
    ]
  }
}
```

**Developer note**

This package contains native code and requires a custom development build. It will not work with Expo Go.

## Example App

For a full working example app, see the [React Native CLI example app](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/react-native).