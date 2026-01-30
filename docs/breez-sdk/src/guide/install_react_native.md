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

**Note:** This package contains native code and requires a custom development build. It will not work with Expo Go.
