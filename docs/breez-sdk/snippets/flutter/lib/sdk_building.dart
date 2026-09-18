import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'dart:async';

Future<void> initSdkAdvanced() async {
  // ANCHOR: init-sdk-advanced
  // Construct the seed using a mnemonic, entropy or passkey
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
  // builder.withAccountNumber(accountNumber: <account number>);
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

Future<void> withAccountNumber(SdkBuilder builder) async {
  // ANCHOR: with-account-number
  var accountNumber = 21;
  builder.withAccountNumber(accountNumber: accountNumber);
  // ANCHOR_END: with-account-number
}

// ANCHOR: with-payment-observer
// ANCHOR_END: with-payment-observer

// ANCHOR: with-session-store
// ANCHOR_END: with-session-store

Future<void> refundPendingConversions(BreezSdk sdk) async {
  // ANCHOR: refund-pending-conversions
  // The flashnet conversion refunder doesn't run in the background in server
  // mode. Call this from your own scheduler (e.g. once per minute) to issue
  // pending refunds for failed conversions.
  await sdk.refundPendingConversions();
  // ANCHOR_END: refund-pending-conversions
}
