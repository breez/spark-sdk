import init, { connect, defaultConfig, initSync, type Config } from '@breeztech/breez-sdk-spark'
import * as web from '@breeztech/breez-sdk-spark/web'
import * as bundler from '@breeztech/breez-sdk-spark/bundler'
import * as ssr from '@breeztech/breez-sdk-spark/ssr'
import * as passkey from '@breeztech/breez-sdk-spark/passkey-prf-provider'
import * as x402Fetch from '@breeztech/breez-sdk-spark/x402-fetch'
import * as x402Server from '@breeztech/breez-sdk-spark/x402-server'

await init()
const config: Config = defaultConfig('regtest')
void config
void connect
void initSync
void web.connect
void bundler.connect
void ssr.connect
void passkey
void x402Fetch.withX402
void x402Server.requirePayment
