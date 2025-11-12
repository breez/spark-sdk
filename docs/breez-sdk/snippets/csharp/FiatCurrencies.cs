using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class FiatCurrencies
    {
        async Task ListFiatCurrencies(BreezSdk sdk)
        {
            // ANCHOR: list-fiat-currencies
            var response = await sdk.ListFiatCurrencies();
            // ANCHOR_END: list-fiat-currencies
        }

        async Task ListFiatRates(BreezSdk sdk)
        {
            // ANCHOR: list-fiat-rates
            var response = await sdk.ListFiatRates();
            // ANCHOR_END: list-fiat-rates
        }
    }
}
