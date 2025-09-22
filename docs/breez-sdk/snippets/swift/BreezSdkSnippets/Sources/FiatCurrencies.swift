import BreezSdkSpark
import Foundation

func listFiatCurrencies(sdk: BreezSdk) async throws -> ListFiatCurrenciesResponse {
    // ANCHOR: list-fiat-currencies
    let response = try await sdk.listFiatCurrencies()
    // ANCHOR_END: list-fiat-currencies
    return response
}

func listFiatRates(sdk: BreezSdk) async throws -> ListFiatRatesResponse {
    // ANCHOR: list-fiat-rates
    let response = try await sdk.listFiatRates()
    // ANCHOR_END: list-fiat-rates
    return response
}
