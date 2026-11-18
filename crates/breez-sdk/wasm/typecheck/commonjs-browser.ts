// In projects that TypeScript compiles as CommonJS, it gives the root import
// the Node types and /web the browser types.
import init, { connect, defaultConfig, initSync, type Config } from '@breeztech/breez-sdk-spark/web'

export async function start(): Promise<void> {
  await init()
  const config: Config = defaultConfig('regtest')
  void config
  void connect
  void initSync
}
