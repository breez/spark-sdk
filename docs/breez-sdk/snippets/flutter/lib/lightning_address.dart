import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Config configureLightningAddress() {
  // ANCHOR: config-lightning-address
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(
        apiKey: 'your-api-key',
        lnurlDomain: 'yourdomain.com'
      );
  // ANCHOR_END: config-lightning-address
  return config;
}

Future<bool> checkLightningAddressAvailability(BreezSdk sdk) async {
  final username = 'myusername';
  
  // ANCHOR: check-lightning-address
  final request = CheckLightningAddressRequest(
    username: username,
  );
  
  final available = await sdk.checkLightningAddressAvailable(request: request);
  // ANCHOR_END: check-lightning-address
  return available;
}

Future<(String, String, String)> registerLightningAddress(BreezSdk sdk) async {
  final username = 'myusername';
  final description = 'My Lightning Address';
  // ANCHOR: register-lightning-address
  final request = RegisterLightningAddressRequest(
    username: username,
    description: description,
  );

  final addressInfo = await sdk.registerLightningAddress(request: request);
  final lightningAddress = addressInfo.lightningAddress;
  final lnurlUrl = addressInfo.lnurl.url;
  final lnurlBech32 = addressInfo.lnurl.bech32;
  // ANCHOR_END: register-lightning-address
  return (lightningAddress, lnurlUrl, lnurlBech32);
}

Future<(String, String, String, String, String)> getLightningAddress(
    BreezSdk sdk) async {
  // ANCHOR: get-lightning-address
  final addressInfoOpt = await sdk.getLightningAddress();

  if (addressInfoOpt == null) {
    throw Exception("No Lightning Address registered for this user.");
  }

  final lightningAddress = addressInfoOpt.lightningAddress;
  final username = addressInfoOpt.username;
  final description = addressInfoOpt.description;
  final lnurlUrl = addressInfoOpt.lnurl.url;
  final lnurlBech32 = addressInfoOpt.lnurl.bech32;
  // ANCHOR_END: get-lightning-address
  return (lightningAddress, username, description, lnurlUrl, lnurlBech32);
}

// Run on the *current owner's* wallet. Produces the authorization that the
// new owner needs to take over the username in a single atomic call.
Future<LightningAddressTransfer> signLightningAddressTransfer(
  BreezSdk currentOwnerSdk,
  String currentOwnerPubkey,
  String newOwnerPubkey,
) async {
  final username = 'myusername';

  // ANCHOR: sign-lightning-address-transfer
  // `username` must be lowercased and trimmed.
  // pubkeys are hex-encoded secp256k1 compressed (via getInfo().identityPubkey).
  final message = 'transfer:$currentOwnerPubkey-$username-$newOwnerPubkey';
  final signed = await currentOwnerSdk.signMessage(
    request: SignMessageRequest(message: message, compact: false),
  );

  final transfer = LightningAddressTransfer(
    pubkey: signed.pubkey,
    signature: signed.signature,
  );
  // ANCHOR_END: sign-lightning-address-transfer
  return transfer;
}

// Run on the *new owner's* wallet with the authorization received
// out-of-band from the current owner.
Future<void> registerLightningAddressViaTransfer(
  BreezSdk newOwnerSdk,
  LightningAddressTransfer transfer,
) async {
  final username = 'myusername';
  final description = 'My Lightning Address';

  // ANCHOR: register-lightning-address-transfer
  final request = RegisterLightningAddressRequest(
    username: username,
    description: description,
    transfer: transfer,
  );

  await newOwnerSdk.registerLightningAddress(request: request);
  // ANCHOR_END: register-lightning-address-transfer
}

Future<void> deleteLightningAddress(BreezSdk sdk) async {
  // ANCHOR: delete-lightning-address
  await sdk.deleteLightningAddress();
  // ANCHOR_END: delete-lightning-address
}

Future<void> accessSenderComment(BreezSdk sdk) async {
  final paymentId = '<payment id>';
  final response = await sdk.getPayment(
    request: GetPaymentRequest(paymentId: paymentId),
  );
  final payment = response.payment;

  // ANCHOR: access-sender-comment
  // Check if this is a lightning payment with LNURL receive metadata
  if (payment.details case PaymentDetails_Lightning lightningDetails) {
    final metadata = lightningDetails.lnurlReceiveMetadata;

    // Access the sender comment if present
    final comment = metadata?.senderComment;
    if (comment != null) {
      print('Sender comment: $comment');
    }
  }
  // ANCHOR_END: access-sender-comment
}

Future<void> accessNostrZap(BreezSdk sdk) async {
  final paymentId = '<payment id>';
  final response = await sdk.getPayment(
    request: GetPaymentRequest(paymentId: paymentId),
  );
  final payment = response.payment;

  // ANCHOR: access-nostr-zap
  // Check if this is a lightning payment with LNURL receive metadata
  if (payment.details case PaymentDetails_Lightning lightningDetails) {
    final metadata = lightningDetails.lnurlReceiveMetadata;

    if (metadata != null) {
      // Access the Nostr zap request if present
      final zapRequest = metadata.nostrZapRequest;
      if (zapRequest != null) {
        // The zapRequest is a JSON string containing the Nostr event (kind 9734)
        print('Nostr zap request: $zapRequest');
      }

      // Access the Nostr zap receipt if present
      final zapReceipt = metadata.nostrZapReceipt;
      if (zapReceipt != null) {
        // The zapReceipt is a JSON string containing the Nostr event (kind 9735)
        print('Nostr zap receipt: $zapReceipt');
      }
    }
  }
  // ANCHOR_END: access-nostr-zap
}
