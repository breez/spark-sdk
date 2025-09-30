import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'helper.dart';
import 'dart:async';

Future<void> initSdk() async {
  // ANCHOR: init-sdk
  // Call once on your Dart entrypoint file, e.g.; `lib/main.dart`
  // or singleton SDK service. It is recommended to use a single instance
  // of the SDK across your Flutter app.
  await BreezSdkSparkLib.init();

  // Construct the seed using mnemonic words or entropy bytes
  String mnemonic = "<mnemonic words>";
  final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);

  // Create the default config
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: "<breez api key>");

  final connectRequest =
      ConnectRequest(config: config, seed: seed, storageDir: "./.data");

  final sdk = await connect(request: connectRequest);
  // ANCHOR_END: init-sdk
  print(sdk);
}

Future<void> initSdkAdvanced() async {
  // ANCHOR: init-sdk-advanced
  // Construct the seed using mnemonic words or entropy bytes
  String mnemonic = "<mnemonic words>";
  final seed = Seed.mnemonic(mnemonic: mnemonic, passphrase: null);

  // Create the default config
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: "<breez api key>");

  // Create the default storage
  final storage = defaultStorage(dataDir: "./.data");

  final builder =
      SdkBuilder(config: config, seed: seed, storage: storage);
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

Future<void> fetchBalance(BreezSdk sdk) async {
  // ANCHOR: fetch-balance
  // forceSync: true will force the SDK to sync with the Spark network
  // before returning the balance
  final info = await sdk.getInfo(request: GetInfoRequest(forceSync: false));
  final balanceSats = info.balanceSats;
  // ANCHOR_END: fetch-balance
  print(balanceSats);
}

class BreezSdkSpark {
  // ANCHOR: logging
  StreamSubscription<LogEntry>? _logSubscription;
  Stream<LogEntry>? _logStream;

  // Initializes SDK log stream.
  //
  // Call once on your Dart entrypoint file, e.g.; `lib/main.dart`
  // or singleton SDK service. It is recommended to use a single instance
  // of the SDK across your Flutter app.
  void initializeLogStream() {
    _logStream ??= initLogging().asBroadcastStream();
  }

  final _logStreamController = StreamController<LogEntry>.broadcast();
  Stream<LogEntry> get logStream => _logStreamController.stream;

  // Subscribe to the log stream
  void subscribeToLogStream() {
    _logSubscription = _logStream?.listen((logEntry) {
      _logStreamController.add(logEntry);
    }, onError: (e) {
      _logStreamController.addError(e);
    });
  }

  // Unsubscribe from the log stream
  void unsubscribeFromLogStream() {
    _logSubscription?.cancel();
  }
  // ANCHOR_END: logging

  // ANCHOR: add-event-listener
  StreamSubscription<SdkEvent>? _eventSubscription;
  Stream<SdkEvent>? _eventStream;

  // Initializes SDK event stream.
  //
  // Call once on your Dart entrypoint file, e.g.; `lib/main.dart`
  // or singleton SDK service. It is recommended to use a single instance
  // of the SDK across your Flutter app.
  void initializeEventsStream(BreezSdk sdk) {
    _eventStream ??= sdk.addEventListener().asBroadcastStream();
  }

  final _eventStreamController = StreamController<SdkEvent>.broadcast();
  Stream<SdkEvent> get eventStream => _eventStreamController.stream;

  // Subscribe to the event stream
  void subscribeToEventStream() {
    _eventSubscription = _eventStream?.listen((sdkEvent) {
      _eventStreamController.add(sdkEvent);
    }, onError: (e) {
      _eventStreamController.addError(e);
    });
  }
  // ANCHOR_END: add-event-listener

  // ANCHOR: remove-event-listener
  void unsubscribeFromEventStream() {
    _eventSubscription?.cancel();
  }
  // ANCHOR_END: remove-event-listener

  // ANCHOR: disconnect
  void disconnect(BreezSdk sdk) {
    sdk.disconnect();
  }
  // ANCHOR_END: disconnect
}
