import { type BreezSdk, defaultConfig } from '@breeztech/breez-sdk-spark'

const configureLightningAddress = () => {
  // ANCHOR: config-lightning-address
  const config = defaultConfig('bitcoin');
  config.apiKey = 'your-api-key';
  config.lnurlDomain = 'yourdomain.com';
  // ANCHOR_END: config-lightning-address
  return config;
}

const exampleCheckLightningAddressAvailability = async (sdk: BreezSdk) => {
  const username = 'myusername'
  
  // ANCHOR: check-lightning-address
  const request = {
    username
  }
  
  const available = await sdk.checkLightningAddressAvailable(request)
  // ANCHOR_END: check-lightning-address
}

const exampleRegisterLightningAddress = async (sdk: BreezSdk) => {
  const username = 'myusername'
  const description = 'My Lightning Address'
  
  // ANCHOR: register-lightning-address
  const request = {
    username,
    description
  }
  
  const addressInfo = await sdk.registerLightningAddress(request)
  const lightningAddress = addressInfo.lightningAddress
  const lnurl = addressInfo.lnurl
  // ANCHOR_END: register-lightning-address
}

const exampleGetLightningAddress = async (sdk: BreezSdk) => {
  // ANCHOR: get-lightning-address
  const addressInfoOpt = await sdk.getLightningAddress()
  
  if (addressInfoOpt) {
    const lightningAddress = addressInfoOpt.lightningAddress
    const username = addressInfoOpt.username
    const description = addressInfoOpt.description
    const lnurl = addressInfoOpt.lnurl
  }
  // ANCHOR_END: get-lightning-address
}

const exampleDeleteLightningAddress = async (sdk: BreezSdk) => {
  // ANCHOR: delete-lightning-address
  await sdk.deleteLightningAddress()
  // ANCHOR_END: delete-lightning-address
}
