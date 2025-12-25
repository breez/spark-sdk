import 'package:breez_sdk_spark/breez_sdk_spark.dart';

// ANCHOR: default-external-signer
Future<ExternalSigner> createSigner() async {
  final mnemonic = '<mnemonic words>';
  final network = Network.mainnet;
  final keySetType = KeySetType.default_;
  final useAddressIndex = false;
  final accountNumber = 0;
  
  final signer = await defaultExternalSigner(
    mnemonic: mnemonic,
    passphrase: null,
    network: network,
    keySetType: keySetType,
    useAddressIndex: useAddressIndex,
    accountNumber: accountNumber,
  );
  
  return signer;
}
// ANCHOR_END: default-external-signer

// ANCHOR: connect-with-signer
Future<BreezSdk> connectWithSigner() async {
  // Create the signer
  final signer = await defaultExternalSigner(
    mnemonic: '<mnemonic words>',
    passphrase: null,
    network: Network.mainnet,
    keySetConfig: KeySetConfig(
      keySetType: KeySetType.default_,
      useAddressIndex: false,
      accountNumber: 0,
    ),
  );
  
  // Create the config
  final config = defaultConfig(Network.mainnet);
  config.apiKey = '<breez api key>';
  
  // Connect using the external signer
  final sdk = await connectWithSigner(ConnectWithSignerRequest(
    config: config,
    signer: signer,
    storageDir: './.data',
  ));
  
  return sdk;
}
// ANCHOR_END: connect-with-signer
