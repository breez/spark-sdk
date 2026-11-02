import sdk, { connect, defaultConfig, type Config } from '@breeztech/breez-sdk-spark'
import * as root from '@breeztech/breez-sdk-spark'
import * as nodejs from '@breeztech/breez-sdk-spark/nodejs'
import * as x402Fetch from '@breeztech/breez-sdk-spark/x402-fetch'
import * as x402Server from '@breeztech/breez-sdk-spark/x402-server'

const config: Config = defaultConfig('regtest')
void config
void connect
void sdk.connect
void nodejs.connect
void x402Fetch.withX402
void x402Server.requirePayment

// @ts-expect-error: initSync is web-only, so Node ESM must not resolve to the web types
void root.initSync
