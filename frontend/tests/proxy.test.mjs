import test from 'node:test'
import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { fileURLToPath } from 'node:url'

const frontend = fileURLToPath(new URL('..', import.meta.url))

test('Vite forwards API host, query, credentials and JSON body without changing upstream status', async t => {
  const upstream = createServer(async (req, res) => {
    const body = []
    for await (const chunk of req) body.push(chunk)
    res.writeHead(409, { 'Content-Type': 'application/json' })
    res.end(JSON.stringify({ path: req.url, method: req.method, authorization: req.headers.authorization, targetHeader: req.headers['x-relayfold-host'] ?? null, body: JSON.parse(Buffer.concat(body).toString()) }))
  })
  await new Promise(resolve => upstream.listen(0, '127.0.0.1', resolve))
  t.after(() => new Promise(resolve => upstream.close(resolve)))
  const vite = spawn(process.execPath, ['node_modules/vite/bin/vite.js', '--host', '127.0.0.1', '--port', '0'], { cwd: frontend, stdio: ['ignore', 'pipe', 'pipe'] })
  t.after(async () => { if (vite.exitCode === null) { vite.kill(); await once(vite, 'exit') } })
  const address = await new Promise((resolve, reject) => {
    let output = ''
    const timeout = setTimeout(() => reject(new Error(`Vite startup timed out: ${output}`)), 10000)
    vite.stdout.on('data', chunk => {
      output += chunk
      const match = output.match(/http:\/\/127\.0\.0\.1:\d+/)
      if (match) { clearTimeout(timeout); resolve(match[0]) }
    })
    vite.stderr.on('data', chunk => { output += chunk })
    vite.on('exit', code => { clearTimeout(timeout); reject(new Error(`Vite exited (${code}): ${output}`)) })
  })
  const response = await fetch(`${address}/api/workflows/run/pause?test=1`, {
    method: 'POST',
    headers: { 'X-RelayFold-Host': `http://127.0.0.1:${upstream.address().port}`, Authorization: 'Bearer test-key', 'Content-Type': 'application/json' },
    body: JSON.stringify({ input: 'hello' }),
  })
  assert.equal(response.status, 409)
  assert.deepEqual(await response.json(), { path: '/workflows/run/pause?test=1', method: 'POST', authorization: 'Bearer test-key', targetHeader: null, body: { input: 'hello' } })
  const invalid = await fetch(`${address}/api/workflows`, { headers: { 'X-RelayFold-Host': 'ftp://localhost' } })
  assert.equal(invalid.status, 400)
  assert.equal((await fetch(address)).status, 200)
})
