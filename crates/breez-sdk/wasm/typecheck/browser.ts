import init, { connect, defaultConfig, initSync, type Config } from '@breeztech/breez-sdk-spark'
import * as web from '@breeztech/breez-sdk-spark/web'
import * as bundler from '@breeztech/breez-sdk-spark/bundler'
import * as ssr from '@breeztech/breez-sdk-spark/ssr'
import * as passkey from '@breeztech/breez-sdk-spark/passkey-prf-provider'

await init()
const config: Config = defaultConfig('regtest')
void config
void connect
void initSync
void web.connect
void bundler.connect
void ssr.connect
void passkey
