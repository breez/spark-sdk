import { type BreezSdk, type LnurlAuthRequestDetails } from '@breeztech/breez-sdk-spark'

const parseLnurlAuth = async (sdk: BreezSdk) => {
  // ANCHOR: parse-lnurl-auth
  // LNURL-auth URL from a service
  // Can be in the form:
  // - lnurl1... (bech32 encoded)
  // - https://service.com/lnurl-auth?tag=login&k1=...
  const lnurlAuthUrl = 'lnurl1...'

  const inputType = await sdk.parse(lnurlAuthUrl)
  if (inputType.type === 'lnurlAuth') {
    console.log(`Domain: ${inputType.domain}`)
    console.log(`Action: ${inputType.action}`)

    // Show domain to user and ask for confirmation
    // This is important for security
  }
  // ANCHOR_END: parse-lnurl-auth
}

const authenticate = async (sdk: BreezSdk, requestData: LnurlAuthRequestDetails) => {
  // ANCHOR: lnurl-auth
  // Perform LNURL authentication
  const result = await sdk.lnurlAuth(requestData)

  if (result.type === 'ok') {
    console.log('Authentication successful')
  } else if (result.type === 'errorStatus') {
    console.log(`Authentication failed: ${result.errorDetails.reason}`)
  }
  // ANCHOR_END: lnurl-auth
}
