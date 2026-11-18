import { connect, defaultConfig, type Config } from '@breeztech/breez-sdk-spark'
import * as root from '@breeztech/breez-sdk-spark'

const config: Config = defaultConfig('regtest')
void config
void connect

// @ts-expect-error: initSync is web-only, so legacy Node resolution must not resolve to the web types
void root.initSync
