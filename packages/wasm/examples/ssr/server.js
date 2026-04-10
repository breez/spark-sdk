// Minimal Vite SSR dev server.
// Based on https://vite.dev/guide/ssr
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import express from 'express'
import { createServer as createViteServer } from 'vite'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

async function start() {
  const app = express()

  const vite = await createViteServer({
    server: { middlewareMode: true },
    appType: 'custom',
    optimizeDeps: {
      exclude: ['@breeztech/breez-sdk-spark'],
    },
  })

  app.use(vite.middlewares)

  app.use('{*path}', async (req, res, next) => {
    try {
      const url = req.originalUrl

      // Read and transform index.html
      let template = fs.readFileSync(path.resolve(__dirname, 'index.html'), 'utf-8')
      template = await vite.transformIndexHtml(url, template)

      // Load the server entry — this is the SSR test:
      // the SDK import must not crash in Node.js.
      const { render } = await vite.ssrLoadModule('/src/entry-server.js')
      const ssrHtml = render()

      // Inject SSR content and client script
      const html = template
        .replace('<!--ssr-outlet-->', ssrHtml)
        .replace('<!--app-script-->', '<script type="module" src="/src/entry-client.js"></script>')

      res.status(200).set({ 'Content-Type': 'text/html' }).end(html)
    } catch (e) {
      vite.ssrFixStacktrace(e)
      next(e)
    }
  })

  const port = 3000
  app.listen(port, () => {
    console.log(`SSR demo running at http://localhost:${port}`)
  })
}

start()
