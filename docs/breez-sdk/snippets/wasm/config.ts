import { defaultConfig, Fee } from '@breeztech/breez-sdk-spark'

const exampleConfigureSdk = async () => {
  // ANCHOR: max-deposit-claim-fee
  // Create the default config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Disable automatic claiming
  config.maxDepositClaimFee = undefined

  // Set a maximum feerate of 10 sat/vB
  config.maxDepositClaimFee = { type: 'rate', satPerVbyte: 10 }

  // Set a maximum fee of 1000 sat
  config.maxDepositClaimFee = { type: 'fixed', amount: 1000 }
  // ANCHOR_END: max-deposit-claim-fee
  console.log('Config:', config)
}

export { exampleConfigureSdk }
