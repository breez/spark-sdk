import { defaultConfig, Network, Fee } from '@breeztech/breez-sdk-spark-react-native'

const exampleConfigureSdk = () => {
  // ANCHOR: max-deposit-claim-fee
  // Create the default config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Disable automatic claiming
  config.maxDepositClaimFee = undefined

  // Set a maximum feerate of 10 sat/vB
  config.maxDepositClaimFee = new Fee.Rate({ satPerVbyte: BigInt(10) })

  // Set a maximum fee of 1000 sat
  config.maxDepositClaimFee = new Fee.Fixed({ amount: BigInt(1000) })
  // ANCHOR_END: max-deposit-claim-fee
  console.log('Config:', config)
}

const exampleConfigurePrivateEnabledDefault = () => {
  // ANCHOR: private-enabled-default
  // Disable Spark private mode by default
  const config = defaultConfig(Network.Mainnet)
  config.privateEnabledDefault = false
  // ANCHOR_END: private-enabled-default
  console.log('Config:', config)
}

export { exampleConfigureSdk, exampleConfigurePrivateEnabledDefault }
