import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> parseInput(BreezSdk sdk) async {
  // ANCHOR: parse-inputs
  String input = "an input to be parsed...";

  InputType inputType = await sdk.parse(input: input);
  if (inputType is BitcoinAddressInputType) {
    print("Input is Bitcoin address ${inputType.v0.address}");
  } else if (inputType is Bolt11InvoiceInputType) {
    String amountStr = inputType.v0.amountMsat != null
        ? inputType.v0.amountMsat.toString()
        : "unknown";
    print("Input is BOLT11 invoice for $amountStr msats");
  } else if (inputType is LnurlPayInputType) {
    print("Input is LNURL-Pay/Lightning address accepting min/max "
        "${inputType.v0.minSendable}/${inputType.v0.maxSendable} msats");
  } else if (inputType is LnurlWithdrawInputType) {
    print("Input is LNURL-Withdraw for min/max "
        "${inputType.v0.minWithdrawable}/${inputType.v0.maxWithdrawable} msats");
  } else if (inputType is SparkAddressInputType) {
    print("Input is Spark address ${inputType.v0.address}");
  } else if (inputType is SparkInvoiceInputType) {
    var invoice = inputType.v0;
    print("Input is Spark invoice:");
    if (invoice.tokenIdentifier != null) {
      print("  Amount: ${invoice.amount} base units of token with id ${invoice.tokenIdentifier}");
    } else {
      print("  Amount: ${invoice.amount} sats");
    }

    if (invoice.description != null) {
      print("  Description: ${invoice.description}");
    }

    if (invoice.expiryTime != null) {
      print("  Expiry time: "
          "${DateTime.fromMillisecondsSinceEpoch(invoice.expiryTime!.toInt() * 1000)}");
    }

    if (invoice.senderPublicKey != null) {
      print("  Sender public key: ${invoice.senderPublicKey}");
    }
  } else if (inputType is CrossChainAddressInputType) {
    var details = inputType.v0;
    print("Input is cross-chain address ${details.address} (${details.addressFamily})");
  } else {
    // Other input types are available
  }
  // ANCHOR_END: parse-inputs
}

Future<void> setExternalInputParsers() async {
  // ANCHOR: set-external-input-parsers
  // Create the default config
  Config config = defaultConfig(network: Network.mainnet)
      .copyWith(apiKey: "<breez api key>");

  config = config.copyWith(
    externalInputParsers: [
      ExternalInputParser(
        providerId: "provider_a",
        inputRegex: "^provider_a",
        parserUrl: "https://parser-domain.com/parser?input=<input>",
      ),
      ExternalInputParser(
        providerId: "provider_b",
        inputRegex: "^provider_b",
        parserUrl: "https://parser-domain.com/parser?input=<input>",
      ),
    ],
  );
  // ANCHOR_END: set-external-input-parsers
}