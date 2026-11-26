import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'dart:async';
import 'dart:typed_data';

Future<void> initSdkAdvanced() async {
  // ANCHOR: init-sdk-advanced
  // Construct the seed using a mnemonic, entropy or passkey
  String mnemonic = "<mnemonic words>";
  final seed = MnemonicSeed(mnemonic: mnemonic, passphrase: null);

  // Create the default config
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: "<breez api key>");

  // Build the SDK using the config, seed and default storage
  final builder = SdkBuilder(config: config, seed: seed);
  await builder.withDefaultStorage(storageDir: "./.data");
  // You can also pass your custom implementations:
  // await builder.withRestChainService(
  //     url: "https://custom.chain.service",
  //     apiType: ChainApiType.mempoolSpace,
  //     credentials: Credentials(
  //         username: "service-username", password: "service-password"));
  // await builder.withAccountNumber(accountNumber: <account number>);
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
  await builder.withRestChainService(
    url: url,
    apiType: chainApiType,
    credentials: optionalCredentials,
  );
  // ANCHOR_END: with-rest-chain-service
}

Future<void> withAccountNumber(SdkBuilder builder) async {
  // ANCHOR: with-account-number
  var accountNumber = 21;
  await builder.withAccountNumber(accountNumber: accountNumber);
  // ANCHOR_END: with-account-number
}

// ANCHOR: with-payment-observer
class ExamplePaymentObserver implements PaymentObserver {
  @override
  Future<void> beforeSend(List<ProvisionalPayment> payments) async {
    for (final payment in payments) {
      print("About to send payment: ${payment.paymentId} "
          "of amount ${payment.amount}");
    }
  }

  @override
  Future<void> afterSend(List<PaymentIdUpdate> updates) async {
    for (final update in updates) {
      print("Token tx broadcast: ${update.provisionalPaymentId} "
          "-> ${update.finalPaymentId}");
    }
  }
}

Future<void> withPaymentObserver(SdkBuilder builder) async {
  final paymentObserver = ExamplePaymentObserver();
  await builder.withPaymentObserver(paymentObserver: paymentObserver);
}
// ANCHOR_END: with-payment-observer

// ANCHOR: with-session-store
class EncryptingSessionStore implements SessionStore {
  EncryptingSessionStore(this.inner);

  final SessionStore inner;

  @override
  Future<Session> getSession(String serviceIdentityKey) async {
    final session = await inner.getSession(serviceIdentityKey);
    // Decrypt session.token and session.refreshToken here before returning it.
    return session;
  }

  @override
  Future<void> setSession(String serviceIdentityKey, Session session) async {
    // Encrypt session.token and session.refreshToken here before persisting it.
    await inner.setSession(serviceIdentityKey, session);
  }
}

// `identity` is the wallet identity public key bytes, used to scope the store.
Future<SdkBuilder> withSessionStore(
    Config config, Seed seed, Uint8List identity) async {
  // Reuse one storage backend for both the SDK storage and the session store.
  final backend = defaultStorage(storageDir: "./.data");

  // Get the session store the backend provides, then wrap it to add encryption.
  final inner = await defaultSessionStore(
      backend: backend, network: config.network, identity: identity);
  final sessionStore = EncryptingSessionStore(inner);

  final builder = SdkBuilder(config: config, seed: seed);
  await builder.withStorageBackend(storage: backend);
  await builder.withSessionStore(sessionStore: sessionStore);
  return builder;
}
// ANCHOR_END: with-session-store

Future<void> refundPendingConversions(BreezSdk sdk) async {
  // ANCHOR: refund-pending-conversions
  // The flashnet conversion refunder doesn't run in the background in server
  // mode. Call this from your own scheduler (e.g. once per minute) to issue
  // pending refunds for failed conversions.
  await sdk.refundPendingConversions();
  // ANCHOR_END: refund-pending-conversions
}
