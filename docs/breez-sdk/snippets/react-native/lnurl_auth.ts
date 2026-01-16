import {
  type BreezSdk,
  InputType_Tags,
  type LnurlAuthRequestDetails,
  LnurlCallbackStatus_Tags
} from '@breeztech/breez-sdk-spark-react-native'

const parseLnurlAuth = async (sdk: BreezSdk) => {
  // ANCHOR: parse-lnurl-auth
  // LNURL-auth URL from a service
  // Can be in the form:
  // - lnurl1... (bech32 encoded)
  // - https://service.com/lnurl-auth?tag=login&k1=...
  const lnurlAuthUrl = 'lnurl1...'

  const inputType = await sdk.parse(lnurlAuthUrl)
  if (inputType.tag === InputType_Tags.LnurlAuth) {
    const requestData = inputType.inner[0]
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

  if (result.tag === LnurlCallbackStatus_Tags.Ok) {
    console.log('Authentication successful')
  } else if (result.tag === LnurlCallbackStatus_Tags.ErrorStatus) {
    console.log(`Authentication failed: ${result.inner.errorDetails.reason}`)
  }
  // ANCHOR_END: lnurl-auth
}
