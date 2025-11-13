import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

TokenIssuer getTokenIssuer(BreezSdk sdk) {
  // ANCHOR: get-issuer-sdk
  TokenIssuer tokenIssuer = sdk.getTokenIssuer();
  // ANCHOR_END: get-issuer-sdk
  return tokenIssuer;
}

Future<TokenMetadata> createToken(TokenIssuer tokenIssuer) async {
  // ANCHOR: create-token
  CreateIssuerTokenRequest request = CreateIssuerTokenRequest(
    name: "My Token",
    ticker: "MTK",
    decimals: 6,
    isFreezable: false,
    maxSupply: BigInt.from(1000000),
  );
  TokenMetadata tokenMetadata =
      await tokenIssuer.createIssuerToken(request: request);
  print("Token identifier: ${tokenMetadata.identifier}");
  // ANCHOR_END: create-token
  return tokenMetadata;
}

Future<Payment> mintTokens(TokenIssuer tokenIssuer) async {
  // ANCHOR: mint-token
  MintIssuerTokenRequest request = MintIssuerTokenRequest(
    amount: BigInt.from(1000),
  );
  Payment payment = await tokenIssuer.mintIssuerToken(request: request);
  // ANCHOR_END: mint-token
  return payment;
}

Future<Payment> burnTokens(TokenIssuer tokenIssuer) async {
  // ANCHOR: burn-token
  BurnIssuerTokenRequest request = BurnIssuerTokenRequest(
    amount: BigInt.from(1000),
  );
  Payment payment = await tokenIssuer.burnIssuerToken(request: request);
  // ANCHOR_END: burn-token
  return payment;
}

Future<TokenMetadata> getTokenMetadata(TokenIssuer tokenIssuer) async {
  // ANCHOR: get-token-metadata
  TokenBalance tokenBalance = await tokenIssuer.getIssuerTokenBalance();
  print("Token balance: ${tokenBalance.balance}");

  TokenMetadata tokenMetadata = await tokenIssuer.getIssuerTokenMetadata();
  print("Token ticker: ${tokenMetadata.ticker}");
  // ANCHOR_END: get-token-metadata
  return tokenMetadata;
}

Future<void> freezeToken(TokenIssuer tokenIssuer) async {
  // ANCHOR: freeze-token
  String sparkAddress = "<spark address>";
  // Freeze the tokens held at the specified Spark address
  FreezeIssuerTokenRequest freezeRequest =
      FreezeIssuerTokenRequest(address: sparkAddress);
  FreezeIssuerTokenResponse freezeResponse =
      await tokenIssuer.freezeIssuerToken(request: freezeRequest);

  // Unfreeze the tokens held at the specified Spark address
  UnfreezeIssuerTokenRequest unfreezeRequest =
      UnfreezeIssuerTokenRequest(address: sparkAddress);
  UnfreezeIssuerTokenResponse unfreezeResponse =
      await tokenIssuer.unfreezeIssuerToken(request: unfreezeRequest);
  // ANCHOR_END: freeze-token
  print("Tokens frozen: $freezeResponse");
  print("Tokens unfrozen: $unfreezeResponse");
}
