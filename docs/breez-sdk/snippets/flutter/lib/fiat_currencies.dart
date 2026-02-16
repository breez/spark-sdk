import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<ListFiatCurrenciesResponse> listFiatCurrencies(BreezClient client) async {
  // ANCHOR: list-fiat-currencies
  ListFiatCurrenciesResponse response = await client.listFiatCurrencies();
  // ANCHOR_END: list-fiat-currencies
  return response;
}

Future<ListFiatRatesResponse> listFiatRates(BreezClient client) async {
  // ANCHOR: list-fiat-rates
  ListFiatRatesResponse response = await client.listFiatRates();
  // ANCHOR_END: list-fiat-rates
  return response;
}
