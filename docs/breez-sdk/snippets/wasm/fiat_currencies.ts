import {
  type Wallet
} from '@breeztech/breez-sdk-spark'

const exampleListCurrencies = async (wallet: Wallet) => {
  // ANCHOR: list-fiat-currencies
  const currencies = await wallet.fiat.currencies()
  // ANCHOR_END: list-fiat-currencies
}

const exampleListRates = async (wallet: Wallet) => {
  // ANCHOR: list-fiat-rates
  const rates = await wallet.fiat.rates()
  // ANCHOR_END: list-fiat-rates
}
