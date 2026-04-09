// This module runs on the CLIENT after hydration.
// It loads the WASM module and exercises the SDK.
import init, { defaultConfig } from '@breeztech/breez-sdk-spark/ssr'

const statusEl = document.getElementById('status')
const infoEl = document.getElementById('client-info')

async function main() {
  try {
    // Load WASM — this is the only step that requires a browser environment.
    await init()

    statusEl.textContent = 'WASM loaded successfully'
    statusEl.className = 'ok'

    // Call a real SDK function to prove it works after init().
    const config = defaultConfig('regtest')
    infoEl.textContent = JSON.stringify(config, null, 2)
  } catch (err) {
    statusEl.textContent = `Error: ${err.message}`
    statusEl.className = 'err'
    infoEl.textContent = err.stack
  }
}

main()
