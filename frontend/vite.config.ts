import { defineConfig, loadEnv } from 'vite'
import type { Plugin } from 'vite'
import type { IncomingHttpHeaders, IncomingMessage, ServerResponse } from 'node:http'
import { request as httpRequest } from 'node:http'
import { request as httpsRequest } from 'node:https'
import { normalizeHost } from './src/api'

// Keep browser requests same-origin; only the frontend server talks to the API.
function apiProxy(): Plugin {
  function middleware(req: IncomingMessage, res: ServerResponse, next: () => void) {
    if (!req.url?.startsWith('/api/')) return next()
    try {
      const host = req.headers['x-relayfold-host']
      if (typeof host !== 'string') throw new Error('Missing API host')
      const target = new URL(normalizeHost(host))
      const headers: IncomingHttpHeaders = { ...req.headers, host: target.host }
      delete headers['x-relayfold-host']
      delete headers.origin
      delete headers.referer
      const upstream = (target.protocol === 'https:' ? httpsRequest : httpRequest)({
        hostname: target.hostname,
        port: target.port || (target.protocol === 'https:' ? 443 : 80),
        path: req.url.slice(4),
        method: req.method,
        headers,
      }, response => {
        res.writeHead(response.statusCode ?? 502, response.headers)
        response.pipe(res)
      })
      upstream.setTimeout(15000, () => upstream.destroy(new Error('API connection timed out')))
      upstream.on('error', () => {
        if (!res.headersSent) res.writeHead(502, { 'Content-Type': 'text/plain' })
        res.end('Cannot reach orchestrator')
      })
      res.on('close', () => upstream.destroy())
      req.pipe(upstream)
    } catch {
      res.writeHead(400, { 'Content-Type': 'text/plain' })
      res.end('Invalid API host')
    }
  }
  return {
    name: 'relayfold-api-proxy',
    configureServer(server) { server.middlewares.use(middleware) },
    configurePreviewServer(server) { server.middlewares.use(middleware) },
  }
}

import react from '@vitejs/plugin-react'
export default defineConfig(({ mode }) => {
  const env = { ...loadEnv(mode, process.cwd(), ''), ...process.env }
  const address = env.RELAYFOLD_PUBLIC_HTTP_ADDR ?? 'localhost:3000'
  const port = address.split(':').at(-1) || '3000'
  return {
    plugins: [react(), apiProxy()],
    define: { 'import.meta.env.VITE_DEFAULT_API_HOST': JSON.stringify(env.VITE_DEFAULT_API_HOST ?? `http://localhost:${port}`) },
  }
})
