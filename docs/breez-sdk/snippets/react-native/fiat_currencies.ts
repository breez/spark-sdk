import { type BreezClient } from '@breeztech/breez-sdk-spark-react-native'

const exampleListCurrencies = async (client: BreezClient) => {
  // ANCHOR: list-fiat-currencies
  const response = await client.fiat().currencies()
  // ANCHOR_END: list-fiat-currencies
}

const exampleListRates = async (client: BreezClient) => {
  // ANCHOR: list-fiat-rates
  const response = await client.fiat().rates()
  // ANCHOR_END: list-fiat-rates
}
