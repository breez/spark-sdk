## Steps to compile the snippets locally

### Building package Using CI

1. Build a react native package

- By running the `Publish` CI in the spark-sdk repository (use dummy binaries)

2. Download the react-native-{VERSION} artifact
3. Unzip the artifact and put the `breez-sdk-spark-react-native.tgz` file in the `snippets/react-native/packages` folder
4. Run `yarn` to install the package

The first few steps above can be done on the CLI with

```shell
mkdir packages
cd packages

wget $(npm view @breeztech/breez-sdk-spark-react-native dist.tarball)
tar xvfz *.tgz
cp package/breez-sdk-spark-react-native.tgz ../packages/
rm -rf package
cd ..
```

### Building package locally

```shell
cargo install cargo-ndk --version 3.5.4
brew install clang-format
cd ../../../../packages/react-native/
yarn --mode=skip-build
npx patch-package
yarn ubrn:build
yarn prepare
```

To use locally-generated bindings:

- Replace `"@breeztech/breez-sdk-spark-react-native": "0.1.8-dev4"` in `package.json` with
  - `"@breeztech/breez-sdk-spark-react-native": "file:./packages/breez-sdk-spark-react-native.tgz"`
- run `yarn`

## Nix

```bash
yarn add @breeztech/breez-sdk-spark-react-native

nix develop

yarn
tsc
yarn run lint
```
