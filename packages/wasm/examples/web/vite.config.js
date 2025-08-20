import { defineConfig } from 'vite'

export default defineConfig({
  server: {
    fs: {
      // Allow serving files from the parent directories
      allow: ['..', '../..'],
    },
  },
  optimizeDeps: {
    exclude: ['@breeztech/breez-sdk-spark']
  },
  build: {
    target: 'esnext'
  }
})
