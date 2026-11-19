import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'dart:async';

Future<void> initSdk() async {
  // ANCHOR: init-sdk
  // Construct the seed using a mnemonic, entropy or passkey
  String mnemonic = "<mnemonic words>";
  final seed = MnemonicSeed(mnemonic: mnemonic, passphrase: null);

  // Create the default config
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: "<breez api key>");

  final connectRequest =
      ConnectRequest(config: config, seed: seed, storageDir: "./.data");

  final sdk = await connect(request: connectRequest);
  // ANCHOR_END: init-sdk
  print(sdk);
}

Future<void> fetchBalance(BreezSdk sdk) async {
  // ANCHOR: fetch-balance
  // ensureSynced: true will ensure the SDK is synced with the Spark network
  // before returning the balance
  final info = await sdk.getInfo(request: GetInfoRequest(ensureSynced: false));
  final identityPubkey = info.identityPubkey;
  final balanceSats = info.balanceSats;
  // ANCHOR_END: fetch-balance
  print(identityPubkey);
  print(balanceSats);
}

// ANCHOR: spark-status
Future<void> gettingStartedSparkStatus() async {
  final sparkStatus = await getSparkStatus(request: GetSparkStatusRequest());

  switch (sparkStatus.status) {
    case ServiceStatus.operational:
      print("Spark is fully operational");
      break;
    case ServiceStatus.degraded:
      print("Spark is experiencing degraded performance");
      break;
    case ServiceStatus.partial:
      print("Spark is partially unavailable");
      break;
    case ServiceStatus.major:
      print("Spark is experiencing a major outage");
      break;
    case ServiceStatus.unknown:
      print("Spark status is unknown");
      break;
  }

  print("Last updated: ${sparkStatus.lastUpdated}");
}
// ANCHOR_END: spark-status

// ANCHOR: logging
class SdkLogger implements Logger {
  final _logStreamController = StreamController<LogEntry>.broadcast();
  Stream<LogEntry> get logStream => _logStreamController.stream;

  @override
  void log(LogEntry l) {
    _logStreamController.add(l);
  }
}

// Call once, before connecting, e.g. in `lib/main.dart`.
void setLogger(SdkLogger logger) {
  initLogging(logDir: null, appLogger: logger, logFilter: null);
}
// ANCHOR_END: logging

// ANCHOR: add-event-listener
class SdkListener implements EventListener {
  final _eventStreamController = StreamController<SdkEvent>.broadcast();
  Stream<SdkEvent> get eventStream => _eventStreamController.stream;

  @override
  Future<void> onEvent(SdkEvent event) async {
    switch (event) {
      case SyncedSdkEvent():
        // Data has been synchronized with the network. When this event is received,
        // it is recommended to refresh the payment list and wallet balance.
        break;
      case NewDepositsSdkEvent(:final newDeposits):
        // Detected deposits, as DepositInfo. Only those with isMature set
        // have enough confirmations to be claimed. Show the rest as pending.
        final _ = newDeposits;
        break;
      case UnclaimedDepositsSdkEvent(:final unclaimedDeposits):
        // Deposits the SDK could not claim. Each claimError says why,
        // most often the fee exceeded the configured maximum.
        final _ = unclaimedDeposits;
        break;
      case ClaimedDepositsSdkEvent(:final claimedDeposits):
        // Deposits claimed into the wallet. An instant (0-conf) claim is
        // reported here on submission and settles shortly after.
        final _ = claimedDeposits;
        break;
      case PaymentSucceededSdkEvent(:final payment):
        // A payment completed. The cached balance is already refreshed,
        // so getInfo returns the new value.
        final _ = payment;
        break;
      case PaymentPendingSdkEvent(:final payment):
        // A payment is awaiting confirmation. It arrives again as
        // succeeded or failed once it settles.
        final _ = payment;
        break;
      case PaymentFailedSdkEvent(:final payment):
        // A payment failed. payment.details carries the method-specific
        // context to show the user.
        final _ = payment;
        break;
      case PaymentMetadataUpdatedSdkEvent(:final payment):
        // Details of a payment already reported changed, such as the
        // conversion info of a cross-chain receive.
        final _ = payment;
        break;
      case AutoOptimizationSdkEvent(:final optimizationEvent):
        // Background optimizer progress: started, round completed, or a
        // terminal outcome. Manual optimizeLeaves calls do not emit these.
        final _ = optimizationEvent;
        break;
      case LightningAddressChangedSdkEvent(:final lightningAddress):
        // The lightning address changed on another device. Unset when the
        // address was deleted.
        final _ = lightningAddress;
        break;
      case UnilateralExitStateChangedSdkEvent():
        // The unilateral exit state changed, so a previously exported
        // one is now out of date. Export it again.
        break;
      case StableBalanceConversionFailedSdkEvent(:final retryInSecs):
        // A stable balance conversion failed. When retryInSecs is set,
        // the SDK retries after it.
        final _ = retryInSecs;
        break;
      case RecoverableFundsSdkEvent(:final recoverableFundsSats):
        // Funds left the balance and wait to be recovered on-chain.
        // Quote their recovery with prepareRecoverFunds.
        final _ = recoverableFundsSats;
        break;
    }
    _eventStreamController.add(event);
  }
}

Future<String> addEventListener(BreezSdk sdk, SdkListener listener) async {
  String listenerId = await sdk.addEventListener(listener: listener);
  return listenerId;
}
// ANCHOR_END: add-event-listener

// ANCHOR: remove-event-listener
Future<void> removeEventListener(BreezSdk sdk, String listenerId) async {
  await sdk.removeEventListener(id: listenerId);
}
// ANCHOR_END: remove-event-listener

// ANCHOR: disconnect
Future<void> disconnect(BreezSdk sdk) async {
  await sdk.disconnect();
}
// ANCHOR_END: disconnect
