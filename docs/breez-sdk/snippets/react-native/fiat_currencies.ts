import { type BreezSdk } from '@breeztech/breez-sdk-spark-react-native'

const exampleListCurrencies = async (sdk: BreezSdk) => {
  // ANCHOR: list-fiat-currencies
  const response = await sdk.listFiatCurrencies()
  // ANCHOR_END: list-fiat-currencies
}

const exampleListRates = async (sdk: BreezSdk) => {
  // ANCHOR: list-fiat-rates
  const response = await sdk.listFiatRates()
  // ANCHOR_END: list-fiat-rates
}
