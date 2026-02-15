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

To enable [Seedless restore](seedless_restore.md#ios-apple-app-site-association) with passkeys, set `enableSeedlessRestore` to `true`. Your app must have the <a target="_blank" href="https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.associated-domains">Associated Domains</a> capability enabled. This adds `webcredentials:keys.breez.technology` to the iOS Associated Domains entitlement:

```json
{
  "expo": {
    "plugins": [
      ["@breeztech/breez-sdk-spark-react-native", {
        "enableSeedlessRestore": true
      }]
    ]
  }
}
```

**Note:** This package contains native code and requires a custom development build. It will not work with Expo Go.
