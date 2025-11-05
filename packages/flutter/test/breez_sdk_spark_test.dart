// ignore_for_file: avoid_print
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';

import 'helper.dart';

void main() {
  setUpAll(() async {
    final lib = await loadExternalLibrary(
      ExternalLibraryLoaderConfig(
        stem: "breez_sdk_spark_flutter",
        ioDirectory: "rust/target/release/",
        webPrefix: null,
      ),
    );
    await BreezSdkSparkLib.init(externalLibrary: lib);
    // Setup logging
    final stream = initLogging();
    stream.listen((logEntry) {
      print('${logEntry.level}: ${logEntry.line}');
    });
  });

  test('Connect', () async {
    String mnemonic =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);

    final config = defaultConfig(network: Network.regtest)
        .copyWith(apiKey: "<api key>");

    final sdkBuilder = SdkBuilder(config: config, seed: seed);
    sdkBuilder.withDefaultStorage(storageDir: './.data');
    final sdk = await sdkBuilder.build();

    /*final sdk = await connect(
      request: ConnectRequest(
        config: config,
        mnemonic: mnemonic,
        storageDir: storageDir,
      ),
    );*/
    sdk.syncWallet(request: SyncWalletRequest());
    // Wait for 10 seconds
    await Future.delayed(const Duration(seconds: 10));

    final info = await sdk.getInfo(request: GetInfoRequest());
    print('Balance: ${info.balanceSats}');
  });
}
