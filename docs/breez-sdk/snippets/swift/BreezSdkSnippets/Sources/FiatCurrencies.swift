import BreezSdkSpark
import Foundation

func listFiatCurrencies(client: BreezClient) async throws -> ListFiatCurrenciesResponse {
    // ANCHOR: list-fiat-currencies
    let response = try await client.listFiatCurrencies()
    // ANCHOR_END: list-fiat-currencies
    return response
}

func listFiatRates(client: BreezClient) async throws -> ListFiatRatesResponse {
    // ANCHOR: list-fiat-rates
    let response = try await client.listFiatRates()
    // ANCHOR_END: list-fiat-rates
    return response
}
