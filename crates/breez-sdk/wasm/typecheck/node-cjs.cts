import { connect, defaultConfig, type Config } from '@breeztech/breez-sdk-spark'
import * as root from '@breeztech/breez-sdk-spark'
import * as nodejs from '@breeztech/breez-sdk-spark/nodejs'

const config: Config = defaultConfig('regtest')
void config
void connect
void nodejs.connect

// @ts-expect-error: initSync is web-only, so Node CJS must not resolve to the web types
void root.initSync
