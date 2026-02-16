import { type BreezClient, type LnurlAuthRequestDetails, parseInput } from '@breeztech/breez-sdk-spark'

const parseLnurlAuth = async () => {
  // ANCHOR: parse-lnurl-auth
  // LNURL-auth URL from a service
  // Can be in the form:
  // - lnurl1... (bech32 encoded)
  // - https://service.com/lnurl-auth?tag=login&k1=...
  const lnurlAuthUrl = 'lnurl1...'

  const inputType = await parseInput(lnurlAuthUrl)
  if (inputType.type === 'lnurlAuth') {
    console.log(`Domain: ${inputType.domain}`)
    console.log(`Action: ${inputType.action}`)

    // Show domain to user and ask for confirmation
    // This is important for security
  }
  // ANCHOR_END: parse-lnurl-auth
}

const authenticate = async (client: BreezClient, requestData: LnurlAuthRequestDetails) => {
  // ANCHOR: lnurl-auth
  // Perform LNURL authentication
  const result = await client.lnurl.auth(requestData)

  if (result.type === 'ok') {
    console.log('Authentication successful')
  } else if (result.type === 'errorStatus') {
    console.log(`Authentication failed: ${result.errorDetails.reason}`)
  }
  // ANCHOR_END: lnurl-auth
}
