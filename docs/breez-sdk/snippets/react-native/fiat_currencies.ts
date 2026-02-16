import { type BreezClient } from '@breeztech/breez-sdk-spark-react-native'

const exampleListCurrencies = async (client: BreezClient) => {
  // ANCHOR: list-fiat-currencies
  const response = await client.listFiatCurrencies()
  // ANCHOR_END: list-fiat-currencies
}

const exampleListRates = async (client: BreezClient) => {
  // ANCHOR: list-fiat-rates
  const response = await client.listFiatRates()
  // ANCHOR_END: list-fiat-rates
}
