// ignore_for_file: avoid_print
import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
//import 'package:test/test.dart';
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
    await RustLib.init(externalLibrary: lib);
    // Setup logging
    final stream = initLogging();
    stream.listen((logEntry) {
      print('${logEntry.level}: ${logEntry.line}');
    });
  });

  test('Connect', () async {
    const storageDir = './.data';
    const mnemonic =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const apiKey = "<api key>";
    final storage = defaultStorage(dataDir: storageDir);    
    final config = defaultConfig(
      network: Network.regtest,
    ).copyWith(apiKey: apiKey);

    final sdkBuilder = SdkBuilder(config: config, mnemonic: mnemonic, storage: storage);
    final sdk = await sdkBuilder.build();

    /*final sdk = await connect(
      request: ConnectRequest(
        config: config,
        mnemonic: mnemonic,
        storageDir: storageDir,
      ),
    );*/
    await sdk.syncWallet(request: SyncWalletRequest());
    // Wait for 10 seconds
    await Future.delayed(const Duration(seconds: 10));

    final info = await sdk.getInfo(request: GetInfoRequest());
    print('Balance: ${info.balanceSats}');
  });
}
