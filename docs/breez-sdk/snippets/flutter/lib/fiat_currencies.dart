import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<ListFiatCurrenciesResponse> listFiatCurrencies(BreezSdk sdk) async {
  // ANCHOR: list-fiat-currencies
  ListFiatCurrenciesResponse response = await sdk.listFiatCurrencies();
  // ANCHOR_END: list-fiat-currencies
  return response;
}

Future<ListFiatRatesResponse> listFiatRates(BreezSdk sdk) async {
  // ANCHOR: list-fiat-rates
  ListFiatRatesResponse response = await sdk.listFiatRates();
  // ANCHOR_END: list-fiat-rates
  return response;
}
