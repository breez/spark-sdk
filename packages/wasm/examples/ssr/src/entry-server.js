// This module runs on the SERVER during SSR.
// The SDK import must not crash here — no WASM, no browser APIs.
import { connect, defaultConfig, BreezSdk } from '@breeztech/breez-sdk-spark/ssr'

export function render() {
  // Prove that the imports resolved without error on the server.
  // We can reference the symbols — we just can't call them yet.
  const exportedNames = [connect, defaultConfig, BreezSdk]
    .map((sym) => sym.name || sym.constructor?.name || 'unknown')

  return `<pre>SSR succeeded — imported ${exportedNames.length} SDK symbols on the server:\n${exportedNames.join(', ')}</pre>`
}
