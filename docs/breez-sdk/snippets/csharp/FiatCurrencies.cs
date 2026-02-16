using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class FiatCurrencies
    {
        async Task ListFiatCurrencies(BreezClient client)
        {
            // ANCHOR: list-fiat-currencies
            var response = await client.Fiat().Currencies();
            // ANCHOR_END: list-fiat-currencies
        }

        async Task ListFiatRates(BreezClient client)
        {
            // ANCHOR: list-fiat-rates
            var response = await client.Fiat().Rates();
            // ANCHOR_END: list-fiat-rates
        }
    }
}
