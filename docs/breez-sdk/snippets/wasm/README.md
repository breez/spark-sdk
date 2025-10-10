## Steps to compile the snippets locally

### Building package Using CI
1. Build a Wasm package
  - By running the publish-all-platforms CI in the breez-sdk-spark repository
2. Download the wasm-{VERSION} artifact 
3. Unzip the artifact and put the `breez-sdk-spark.tgz` file in the `snippets/wasm/packages` folder
4. Run `yarn` to install the package.
5. Happy coding

The first few steps above can be done on the CLI with

```shell
mkdir packages
cd packages

wget $(npm view @breeztech/breez-sdk-spark dist.tarball)
tar xvfz *.tgz
cp package/breez-sdk-spark.tgz ../packages/
rm -rf package
cd ..
```

### Building package locally
```shell
cd ../../../../packages/wasm/
make build
yarn pack
```

To use published bindings:
- Replace `"@breeztech/breez-sdk-spark": "file:./packages/breez-sdk-spark.tgz"` in `package.json` with
  - `"@breeztech/breez-sdk-spark": "<package-version>"`
- run `yarn`

## Nix

```
yarn add @breeztech/breez-sdk-spark

nix develop

yarn
tsc
yarn run lint
```