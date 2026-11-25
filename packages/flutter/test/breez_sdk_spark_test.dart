// ignore_for_file: avoid_print
import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'package:flutter_test/flutter_test.dart';

import '../hook/assets.dart';

class FakePrfProvider implements PrfProvider {
  FakePrfProvider({required this.supported, required this.association, this.throwUndeclared = false});

  final bool supported;
  final DomainAssociation association;
  final bool throwUndeclared;

  @override
  Future<bool> isSupported() async {
    if (throwUndeclared) throw StateError('not a PrfProviderException');
    return supported;
  }

  @override
  Future<DomainAssociation> checkDomainAssociation() async => association;

  @override
  Future<DeriveSeedsOutput> deriveSeeds(DeriveSeedsRequest request) =>
      throw PrfNotSupportedPrfProviderException();

  @override
  Future<CreatePasskeyOutput> createPasskey(List<Uint8List> excludeCredentials, List<String> salts) =>
      throw PrfNotSupportedPrfProviderException();
}

class TestLogger implements Logger {
  final lines = <String>[];

  @override
  void log(LogEntry l) => lines.add('${l.level}: ${l.line}');
}

class TestEventListener implements EventListener {
  final synced = Completer<void>();

  @override
  Future<void> onEvent(SdkEvent event) async {
    if (event is SyncedSdkEvent && !synced.isCompleted) synced.complete();
  }
}

final logger = TestLogger();

void main() {
  setUpAll(() => initLogging(logDir: null, appLogger: logger, logFilter: null));

  test('the hook bundles the asset ids the bindings load', () {
    final bindings = File('lib/src/generated/breez_sdk_spark.dart').readAsStringSync();
    final runtime = File('lib/src/generated/uniffi_runtime.dart').readAsStringSync();
    expect(bindings, contains('"package:breez_sdk_spark_flutter/$bindingsAsset"'));
    expect(runtime, contains("'package:breez_sdk_spark_flutter/$relayAsset'"));
  });

  test('default config can be copied with changes', () {
    final config = defaultConfig(network: Network.regtest).copyWith(apiKey: 'key');
    expect(config.network, Network.regtest);
    expect(config.apiKey, 'key');
  });

  test('records and variants compare by value', () {
    final config = defaultConfig(network: Network.regtest);
    final same = defaultConfig(network: Network.regtest);
    expect(identical(config, same), isFalse);
    expect(config, same);
    expect(config.hashCode, same.hashCode);
    expect({config, same}, hasLength(1));
    expect(config, isNot(defaultConfig(network: Network.mainnet)));

    expect(
      ListPaymentsRequest(typeFilter: [PaymentType.send], assetFilter: TokenAssetFilter('id')),
      ListPaymentsRequest(typeFilter: [PaymentType.send], assetFilter: TokenAssetFilter('id')),
    );
    expect(ListPaymentsRequest(typeFilter: [PaymentType.send]), isNot(ListPaymentsRequest(typeFilter: [])));

    final entropy = EntropySeed(Uint8List.fromList([1, 2, 3]));
    expect(entropy, EntropySeed(Uint8List.fromList([1, 2, 3])));
    expect(entropy, isNot(EntropySeed(Uint8List.fromList([1, 2, 4]))));
    expect(entropy, isNot(MnemonicSeed(mnemonic: 'words', passphrase: null)));
  });

  test('enums with fields are sealed', () {
    // Compiles only because Seed is sealed: a switch expression with no
    // fallback must cover every variant.
    String kind(Seed seed) => switch (seed) {
      MnemonicSeed() => 'mnemonic',
      EntropySeed() => 'entropy',
    };
    expect(kind(EntropySeed(Uint8List(32))), 'entropy');
  });

  test('a Dart PrfProvider is called back from Rust', () async {
    final unsupported = PasskeyClient(
      prfProvider: FakePrfProvider(supported: false, association: AssociatedDomainAssociation()),
      breezApiKey: null,
      config: null,
    );
    expect(await unsupported.checkAvailability(), isA<PrfUnsupportedPasskeyAvailability>());

    final notAssociated = PasskeyClient(
      prfProvider: FakePrfProvider(
        supported: true,
        association: NotAssociatedDomainAssociation(source: 'aasa', reason: 'missing entry'),
      ),
      breezApiKey: null,
      config: null,
    );
    final availability = await notAssociated.checkAvailability();
    expect(availability, isA<NotAssociatedPasskeyAvailability>());
    availability as NotAssociatedPasskeyAvailability;
    expect(availability.source, 'aasa');
    expect(availability.reason, 'missing entry');
  });

  test('an undeclared exception from a Dart callback reaches the caller as an SDK error', () async {
    final client = PasskeyClient(
      prfProvider: FakePrfProvider(
        supported: true,
        association: AssociatedDomainAssociation(),
        throwUndeclared: true,
      ),
      breezApiKey: null,
      config: null,
    );
    await expectLater(
      client.checkAvailability(),
      throwsA(
        isA<PrfPasskeyException>().having(
          (e) => e.v0,
          'v0',
          isA<GenericPrfProviderException>().having(
            (e) => e.v0,
            'v0',
            contains('not a PrfProviderException'),
          ),
        ),
      ),
    );
  });

  // Connects to the public regtest network, so it only runs when asked.
  test(
    'connect, sync and receive events and logs from SDK threads',
    () async {
      final storageDir = Directory.systemTemp.createTempSync('breez_sdk_spark_test');
      final listener = TestEventListener();
      final sdk = await connect(
        request: ConnectRequest(
          config: defaultConfig(network: Network.regtest),
          seed: EntropySeed(Uint8List.fromList(List.generate(32, (i) => i))),
          storageDir: storageDir.path,
        ),
      );
      try {
        await sdk.addEventListener(listener: listener);
        await listener.synced.future.timeout(const Duration(minutes: 2));
        final info = await sdk.getInfo(request: GetInfoRequest(ensureSynced: false));
        print('Balance: ${info.balanceSats}');
        expect(logger.lines, isNotEmpty);

        // u128 has no Dart range check: Rust rejects the value as invalid input.
        await expectLater(
          sdk.prepareSendPayment(
            request: PrepareSendPaymentRequest(
              paymentRequest: InputPaymentRequest('x'),
              amount: BigInt.from(-5),
            ),
          ),
          throwsA(isA<InvalidInputSdkException>()),
        );
      } finally {
        await sdk.disconnect();
        storageDir.deleteSync(recursive: true);
      }
    },
    skip: Platform.environment['BREEZ_REGTEST'] == null ? 'set BREEZ_REGTEST=1 to run' : false,
    timeout: const Timeout(Duration(minutes: 3)),
  );
}
