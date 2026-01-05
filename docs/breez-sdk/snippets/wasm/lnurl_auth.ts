import {
  BreezSdk,
  InputType,
  LnurlAuthRequestDetails,
  LnurlCallbackStatus,
} from '@breeztech/breez-sdk-spark-wasm'

const parseLnurlAuth = async (sdk: BreezSdk) => {
  // ANCHOR: parse-lnurl-auth
  // LNURL-auth URL from a service
  // Can be in the form:
  // - lnurl1... (bech32 encoded)
  // - https://service.com/lnurl-auth?tag=login&k1=...
  const lnurlAuthUrl = 'lnurl1...'

  const inputType = await sdk.parse(lnurlAuthUrl)
  if (inputType.type === InputType.LNURL_AUTH) {
    const requestData = inputType.data
    console.log(`Domain: ${requestData.domain}`)
    console.log(`Action: ${requestData.action}`)

    // Show domain to user and ask for confirmation
    // This is important for security
  }
  // ANCHOR_END: parse-lnurl-auth
}

const authenticate = async (sdk: BreezSdk, requestData: LnurlAuthRequestDetails) => {
  // ANCHOR: lnurl-auth
  // Perform LNURL authentication
  const result = await sdk.lnurlAuth(requestData)

  if (result.type === LnurlCallbackStatus.OK) {
    console.log('Authentication successful')
  } else if (result.type === LnurlCallbackStatus.ERROR_STATUS) {
    console.log(`Authentication failed: ${result.data.reason}`)
  }
  // ANCHOR_END: lnurl-auth
}
