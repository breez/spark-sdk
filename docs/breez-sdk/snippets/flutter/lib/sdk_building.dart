import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'helper.dart';
import 'dart:async';

Future<void> initSdkAdvanced() async {
  // ANCHOR: init-sdk-advanced
  // Construct the seed using mnemonic words or entropy bytes
  String mnemonic = "<mnemonic words>";
  final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);

  // Create the default config
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: "<breez api key>");

  // Build the SDK using the config, seed and default storage
  final builder = SdkBuilder(config: config, seed: seed);
  builder.withDefaultStorage(storageDir: "./.data");
  // You can also pass your custom implementations:
  // builder.withRestChainService(
  //     url: "https://custom.chain.service",
  //     credentials: Credentials(
  //         username: "service-username", password: "service-password"));
  // builder.withKeySet(keySetType: <your key set type>, useAddressIndex: <use address index>, accountNumber: <account number>);
  final sdk = await builder.build();
  // ANCHOR_END: init-sdk-advanced
  print(sdk);
}

Future<void> withRestChainService(SdkBuilder builder) async {
  // ANCHOR: with-rest-chain-service
  String url = "<your REST chain service URL>";
  var chainApiType = ChainApiType.mempoolSpace;
  var optionalCredentials = Credentials(
    username: "<username>",
    password: "<password>",
  );
  builder.withRestChainService(
    url: url,
    apiType: chainApiType,
    credentials: optionalCredentials,
  );
  // ANCHOR_END: with-rest-chain-service
}

Future<void> withKeySet(SdkBuilder builder) async {
  // ANCHOR: with-key-set
  var keySetType = KeySetType.default_;
  var useAddressIndex = false;
  var optionalAccountNumber = 21;
  builder.withKeySet(
    keySetType: keySetType,
    useAddressIndex: useAddressIndex,
    accountNumber: optionalAccountNumber,
  );
  // ANCHOR_END: with-key-set
}

// ANCHOR: with-payment-observer
// ANCHOR_END: with-payment-observer
