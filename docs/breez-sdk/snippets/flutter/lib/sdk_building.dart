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
  // builder.withKeySet(config: KeySetConfig(keySetType: <your key set type>, useAddressIndex: <use address index>, accountNumber: <account number>));
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
    config: KeySetConfig(
      keySetType: keySetType,
      useAddressIndex: useAddressIndex,
      accountNumber: optionalAccountNumber,
    ),
  );
  // ANCHOR_END: with-key-set
}

// ANCHOR: with-payment-observer
// ANCHOR_END: with-payment-observer

Future<BreezSdk> initSdkServer() async {
  // ANCHOR: init-sdk-server
  // Construct the seed using a mnemonic, entropy or passkey
  String mnemonic = "<mnemonic words>";
  final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);

  // Build a server-mode config: same as defaultConfig(network) with
  // backgroundTasksEnabled = false. No periodic sync, no real-time sync
  // client, no leaf/token optimizer, no flashnet refunder, no lightning-
  // address recovery, no spark private-mode init.
  final config = defaultServerConfig(network: Network.mainnet)
      .copyWith(apiKey: "<breez api key>");

  // Typically server-mode SDKs are built per request and share infrastructure
  // (DB pool, REST chain service, SSP/Connection Manager) across instances.
  // Pass the shared resources via the builder.
  final builder = SdkBuilder(config: config, seed: seed);
  builder.withDefaultStorage(storageDir: "./.data");
  final sdk = await builder.build();
  // ANCHOR_END: init-sdk-server
  return sdk;
}

Future<String> serverModeRequestHandler(BreezSdk sdk) async {
  // ANCHOR: server-mode-request-handler
  // User-facing request handler: do not call syncWallet here. Operations
  // that read from local storage (getInfo, listPayments, etc.) do not need
  // a defensive sync. Call syncWallet only from webhook handlers or
  // reconciliation jobs that need to observe an external state change.
  final response = await sdk.receivePayment(
    request: ReceivePaymentRequest(
      paymentMethod: ReceivePaymentMethod.bolt11Invoice(
        description: "<invoice description>",
        amountSats: BigInt.from(5000),
        expirySecs: 3600,
        paymentHash: null,
      ),
    ),
  );

  // Always disconnect at the end of the request lifecycle to flush
  // outstanding storage writes.
  await sdk.disconnect();
  // ANCHOR_END: server-mode-request-handler
  return response.paymentRequest;
}

Future<void> serverModeProvisioning(BreezSdk sdk) async {
  // ANCHOR: server-mode-provisioning
  // One-time setup when a wallet is first registered. The client-mode SDK
  // would normally apply the private-mode preset itself on first startup;
  // server-mode SDKs do not, so opt in once here via updateUserSettings.
  await sdk.updateUserSettings(
    request: UpdateUserSettingsRequest(sparkPrivateModeEnabled: true),
  );

  await sdk.disconnect();
  // ANCHOR_END: server-mode-provisioning
}

Future<void> refundPendingConversions(BreezSdk sdk) async {
  // ANCHOR: refund-pending-conversions
  // The flashnet conversion refunder doesn't run in the background in server
  // mode. Call this from your own scheduler (e.g. once per minute) to issue
  // pending refunds for failed conversions.
  await sdk.refundPendingConversions();
  // ANCHOR_END: refund-pending-conversions
}
