import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

BreezIssuerSdk getIssuerSdk(BreezSdk sdk) {
  // ANCHOR: get-issuer-sdk
  BreezIssuerSdk issuerSdk = sdk.getIssuerSdk();
  // ANCHOR_END: get-issuer-sdk
  return issuerSdk;
}

Future<TokenMetadata> createToken(BreezIssuerSdk issuerSdk) async {
  // ANCHOR: create-token
  CreateIssuerTokenRequest request = CreateIssuerTokenRequest(
    name: "My Token",
    ticker: "MTK",
    decimals: 6,
    isFreezable: false,
    maxSupply: BigInt.from(1000000),
  );
  TokenMetadata tokenMetadata = await issuerSdk.createIssuerToken(request: request);
  print("Token identifier: ${tokenMetadata.identifier}");
  // ANCHOR_END: create-token
  return tokenMetadata;
}

Future<Payment> mintTokens(BreezIssuerSdk issuerSdk) async {
  // ANCHOR: mint-token
  MintIssuerTokenRequest request = MintIssuerTokenRequest(
    amount: BigInt.from(1000),
  );
  Payment payment = await issuerSdk.mintIssuerToken(request: request);
  // ANCHOR_END: mint-token
  return payment;
}

Future<Payment> burnTokens(BreezIssuerSdk issuerSdk) async {
  // ANCHOR: burn-token
  BurnIssuerTokenRequest request = BurnIssuerTokenRequest(
    amount: BigInt.from(1000),
  );
  Payment payment = await issuerSdk.burnIssuerToken(request: request);
  // ANCHOR_END: burn-token
  return payment;
}

Future<TokenMetadata> getTokenMetadata(BreezIssuerSdk issuerSdk) async {
  // ANCHOR: get-token-metadata
  GetIssuerTokenBalanceResponse tokenBalance =
      await issuerSdk.getIssuerTokenBalance();
  print("Token balance: ${tokenBalance.balance}");

  TokenMetadata tokenMetadata = await issuerSdk.getIssuerTokenMetadata();
  print("Token ticker: ${tokenMetadata.ticker}");
  // ANCHOR_END: get-token-metadata
  return tokenMetadata;
}

Future<void> freezeToken(BreezIssuerSdk issuerSdk) async {
  // ANCHOR: freeze-token
  String sparkAddress = "<spark address>";
  // Freeze the tokens held at the specified Spark address
  FreezeIssuerTokenRequest freezeRequest =
      FreezeIssuerTokenRequest(address: sparkAddress);
  FreezeIssuerTokenResponse freezeResponse =
      await issuerSdk.freezeIssuerToken(request: freezeRequest);

  // Unfreeze the tokens held at the specified Spark address
  UnfreezeIssuerTokenRequest unfreezeRequest =
      UnfreezeIssuerTokenRequest(address: sparkAddress);
  UnfreezeIssuerTokenResponse unfreezeResponse =
      await issuerSdk.unfreezeIssuerToken(request: unfreezeRequest);
  // ANCHOR_END: freeze-token
  print("Tokens frozen: $freezeResponse");
  print("Tokens unfrozen: $unfreezeResponse");
}