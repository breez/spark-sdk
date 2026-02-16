import {
  type BreezClient
} from '@breeztech/breez-sdk-spark'

const exampleListCurrencies = async (client: BreezClient) => {
  // ANCHOR: list-fiat-currencies
  const currencies = await client.fiat.currencies()
  // ANCHOR_END: list-fiat-currencies
}

const exampleListRates = async (client: BreezClient) => {
  // ANCHOR: list-fiat-rates
  const rates = await client.fiat.rates()
  // ANCHOR_END: list-fiat-rates
}
