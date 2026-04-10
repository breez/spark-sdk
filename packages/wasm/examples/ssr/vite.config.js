import { defineConfig } from 'vite'

export default defineConfig({
  server: {
    fs: {
      allow: ['..', '../..'],
    },
  },
  optimizeDeps: {
    exclude: ['@breeztech/breez-sdk-spark'],
  },
  ssr: {
    // Force Vite to bundle the SDK into the SSR output (not externalize it).
    // This simulates what Next.js/Turbopack does and is the strictest test
    // of the SSR shim — the stubs must be side-effect-free and import-safe.
    noExternal: ['@breeztech/breez-sdk-spark'],
  },
  build: {
    target: 'esnext',
  },
})
