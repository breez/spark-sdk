import {
  defaultConfig,
  Network,
  MaxFee,
  OptimizationConfig
} from '@breeztech/breez-sdk-spark-react-native'

const exampleConfigureSdk = () => {
  // ANCHOR: max-deposit-claim-fee
  // Create the default config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Disable automatic claiming
  config.maxDepositClaimFee = undefined

  // Set a maximum feerate of 10 sat/vB
  config.maxDepositClaimFee = new MaxFee.Rate({ satPerVbyte: BigInt(10) })

  // Set a maximum fee of 1000 sat
  config.maxDepositClaimFee = new MaxFee.Fixed({ amount: BigInt(1000) })

  // Set the maximum fee to the fastest network recommended fee at the time of claim
  // with a leeway of 1 sats/vbyte
  config.maxDepositClaimFee = new MaxFee.NetworkRecommended({ leewaySatPerVbyte: BigInt(1) })
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

const exampleConfigureOptimizationConfiguration = () => {
  // ANCHOR: optimization-configuration
  const config = defaultConfig(Network.Mainnet)
  config.optimizationConfig = { autoEnabled: true, multiplicity: 1 }
  // ANCHOR_END: optimization-configuration
  console.log('Config:', config)
}

const exampleConfigureNoInvoicePaidSupport = () => {
  // ANCHOR: no-invoice-paid-support
  // Disable invoice paid notifications to LNURL server
  const config = defaultConfig(Network.Mainnet)
  config.noInvoicePaidSupport = true
  // ANCHOR_END: no-invoice-paid-support
  console.log('Config:', config)
}

export {
  exampleConfigureSdk,
  exampleConfigurePrivateEnabledDefault,
  exampleConfigureOptimizationConfiguration,
  exampleConfigureNoInvoicePaidSupport
}
